using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Text;
using System.Xml;

namespace Wideband
{
    // Loads a mutable ROM and binds a TunerPro XDF definition to it.
    // Toyota .cuw text is stripped. The calibration bytes stay editable.
    sealed class RomFile
    {
        public readonly MemoryImage Image;
        public readonly long Origin;
        public readonly string Note;

        public RomFile(MemoryImage image, long origin, string note)
        {
            if (image == null) throw new ArgumentNullException("image");
            Image = image;
            Origin = origin;
            Note = note == null ? "" : note;
        }
    }

    static class CalibrationFiles
    {
        public const int MaxCuwSpan = 8 * 1024 * 1024;

        public static RomFile Open(string path)
        {
            if (path == null || path.Trim().Length == 0) throw new ArgumentException("The image path is empty.");
            if (path.EndsWith(".enc", StringComparison.OrdinalIgnoreCase))
                throw new InvalidDataException("Encrypted definition files are not opened.");
            byte[] data = File.ReadAllBytes(path);
            if (data.Length >= 8 && Encoding.ASCII.GetString(data, 0, 8) == "VFTBLPE1")
                throw new InvalidDataException("This profile body is opaque and is not opened.");
            if (path.EndsWith(".cuw", StringComparison.OrdinalIgnoreCase))
                return CuwFile.Read(data);
            return new RomFile(new MemoryImage(data), 0, "");
        }
    }

    static class CuwFile
    {
        public static RomFile Read(byte[] data)
        {
            if (data == null || data.Length == 0)
                throw new InvalidDataException("The CUW file has no calibration payload.");
            int start = 0;
            while (start < data.Length && data[start] == 0) start++;
            if (start >= data.Length)
                throw new InvalidDataException("The CUW file has no calibration payload.");
            byte[] body = data;
            if (start > 0)
            {
                body = new byte[data.Length - start];
                Buffer.BlockCopy(data, start, body, 0, body.Length);
            }
            string latin = Encoding.GetEncoding(28591).GetString(body);
            string[] lines = latin.Split('\n');
            int first = -1;
            for (int i = 0; i < lines.Length; i++)
            {
                if (IsSRecordShape(CleanLine(lines[i])))
                {
                    first = i;
                    break;
                }
            }
            if (first >= 0) return ParseRecords(lines, first);
            return ParseRemainder(body);
        }

        static RomFile ParseRemainder(byte[] body)
        {
            int index = 0;
            while (index < body.Length && IsHeaderByte(body[index])) index++;
            if (index >= body.Length || body.Length - index < 16)
                throw new InvalidDataException("The CUW file has no calibration payload.");
            byte[] payload = new byte[body.Length - index];
            Buffer.BlockCopy(body, index, payload, 0, payload.Length);
            string note = CleanNote(Encoding.GetEncoding(28591).GetString(body, 0, index));
            return new RomFile(new MemoryImage(payload), 0, note);
        }

        static RomFile ParseRecords(string[] lines, int first)
        {
            var header = new StringBuilder();
            for (int i = 0; i < first; i++)
            {
                if (header.Length > 0) header.Append('\n');
                header.Append(lines[i].TrimEnd('\r'));
            }
            var placed = new List<byte[]>();
            var addresses = new List<long>();
            int dataRecords = 0;
            int declared = -1;
            for (int i = first; i < lines.Length; i++)
            {
                string line = CleanLine(lines[i]);
                if (line.Length == 0) continue;
                if (!IsSRecordShape(line))
                    throw new InvalidDataException("The CUW payload has an invalid S-record on line " + (i + 1) + ".");
                ParsedRecord record = ParseLine(line, i + 1);
                if (record.Type == '1' || record.Type == '2' || record.Type == '3')
                {
                    placed.Add(record.Data);
                    addresses.Add(record.Address);
                    dataRecords++;
                }
                else if (record.Type == '5')
                {
                    declared = (int)record.Address;
                }
            }
            if (declared >= 0 && declared != dataRecords)
                throw new InvalidDataException("The CUW S5 count does not match the data records.");
            if (placed.Count == 0)
                throw new InvalidDataException("The CUW file has no calibration payload.");
            long min = addresses[0];
            long maxEnd = addresses[0] + placed[0].Length;
            for (int i = 1; i < addresses.Count; i++)
            {
                if (addresses[i] < min) min = addresses[i];
                long end = addresses[i] + placed[i].Length;
                if (end > maxEnd) maxEnd = end;
            }
            if (min < 0 || maxEnd < min || maxEnd - min > CalibrationFiles.MaxCuwSpan)
                throw new InvalidDataException("The CUW payload is larger than 8 MB.");
            int span = (int)(maxEnd - min);
            if (span < 1) throw new InvalidDataException("The CUW file has no calibration payload.");
            byte[] image = new byte[span];
            for (int i = 0; i < image.Length; i++) image[i] = 0xFF;
            for (int i = 0; i < placed.Count; i++)
            {
                int offset = (int)(addresses[i] - min);
                Buffer.BlockCopy(placed[i], 0, image, offset, placed[i].Length);
            }
            return new RomFile(new MemoryImage(image), min, CleanNote(header.ToString()));
        }

        static ParsedRecord ParseLine(string line, int number)
        {
            char type = char.ToUpperInvariant(line[1]);
            int addressBytes = AddressBytes(type);
            if (addressBytes == 0)
                throw new InvalidDataException("The CUW payload has an invalid S-record on line " + number + ".");
            byte[] bytes;
            try { bytes = Hex(line.Substring(2)); }
            catch (FormatException)
            {
                throw new InvalidDataException("The CUW payload has an invalid S-record on line " + number + ".");
            }
            if (bytes.Length < 1 || bytes.Length != bytes[0] + 1)
                throw new InvalidDataException("The CUW payload has an invalid S-record on line " + number + ".");
            int sum = 0;
            for (int i = 0; i < bytes.Length; i++) sum = (sum + bytes[i]) & 0xFF;
            if (sum != 0xFF)
                throw new InvalidDataException("S-record checksum failed on line " + number + ".");
            int count = bytes[0];
            if (count < addressBytes + 1)
                throw new InvalidDataException("The CUW payload has an invalid S-record on line " + number + ".");
            int dataBytes = count - addressBytes - 1;
            long address = 0;
            for (int i = 0; i < addressBytes; i++) address = (address << 8) | bytes[1 + i];
            byte[] payload = new byte[dataBytes];
            if (dataBytes > 0) Buffer.BlockCopy(bytes, 1 + addressBytes, payload, 0, dataBytes);
            var record = new ParsedRecord();
            record.Type = type;
            record.Address = address;
            record.Data = payload;
            return record;
        }

        static int AddressBytes(char type)
        {
            if (type == '0' || type == '1' || type == '5' || type == '9') return 2;
            if (type == '2' || type == '8') return 3;
            if (type == '3' || type == '7') return 4;
            return 0;
        }

        static byte[] Hex(string text)
        {
            if ((text.Length & 1) != 0) throw new FormatException("odd");
            byte[] data = new byte[text.Length / 2];
            for (int i = 0; i < data.Length; i++) data[i] = Convert.ToByte(text.Substring(i * 2, 2), 16);
            return data;
        }

        static string CleanLine(string line)
        {
            if (line == null) return "";
            return line.Trim('\r', ' ', '\t');
        }

        static bool IsSRecordShape(string line)
        {
            if (line.Length < 4) return false;
            if (line[0] != 'S' && line[0] != 's') return false;
            if (line[1] < '0' || line[1] > '9') return false;
            for (int i = 2; i < line.Length; i++)
            {
                char c = line[i];
                bool hex = (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F');
                if (!hex) return false;
            }
            return true;
        }

        static bool IsHeaderByte(byte value)
        {
            return value == 0x09 || value == 0x0A || value == 0x0D || (value >= 0x20 && value <= 0x7E);
        }

        static string CleanNote(string text)
        {
            if (text == null) return "";
            var builder = new StringBuilder();
            foreach (char c in text)
            {
                if (c == '\n' || c == '\r' || c == '\t' || (c >= ' ' && c <= '~')) builder.Append(c);
                if (builder.Length >= 2000) break;
            }
            return builder.ToString().Trim();
        }

        sealed class ParsedRecord
        {
            public char Type;
            public long Address;
            public byte[] Data = new byte[0];
        }
    }

    sealed class XdfRegion
    {
        public long Address;
        public bool HasAddress;
        public int Bits = 8;
        public int Rows = 1;
        public int Cols = 1;
        public bool RowsSpecified;
        public bool ColsSpecified;
        public int MajorBits;
        public int MinorBits;
        public bool Signed;
        public bool LittleEndian;
        public bool Float;

        public int MinorStride()
        {
            if (MinorBits != 0) return MinorBits;
            return Bits;
        }

        public int MajorStride()
        {
            if (MajorBits != 0) return MajorBits;
            return MinorStride() * (Cols < 1 ? 1 : Cols);
        }
    }

    sealed class XdfSeries
    {
        public XdfRegion Region;
        public ScaleEquation Math;
        public double[] Fixed;
        public int Count;
    }

    sealed class XdfBinding
    {
        public ScaleEquation Conversion;
        public XdfRegion Region;
        public bool Flag;
        public ulong Mask;
        public long Origin;
        public long BaseOffset;
        public long BaseSubtract;
        public bool Transposed;
        public XdfSeries AxisX;
        public XdfSeries AxisY;
        public bool HasMin;
        public bool HasMax;
        public double Min;
        public double Max;

        public void ReadInto(byte[] image, TuneTable table)
        {
            if (table == null) throw new ArgumentNullException("table");
            if (AxisX != null && AxisX.Region != null)
                table.X = ReadSeries(image, AxisX, table.X.Length);
            if (AxisY != null && AxisY.Region != null && !Transposed && table.Kind == "map")
                table.Y = ReadSeries(image, AxisY, table.Y.Length);
            if (Flag)
            {
                bool on = (XdfMemory.ReadRaw(image, this, Region, 0, 0) & Mask) != 0;
                table.Values[0][0] = on ? 1 : 0;
                return;
            }
            if (Transposed)
            {
                for (int row = 0; row < Region.Rows; row++)
                    table.Values[0][row] = XdfMemory.ReadScaled(image, this, Region, Conversion, row, 0);
                return;
            }
            for (int row = 0; row < table.Values.Length; row++)
                for (int col = 0; col < table.Values[row].Length; col++)
                    table.Values[row][col] = XdfMemory.ReadScaled(image, this, Region, Conversion, row, col);
        }

        public void WriteTable(byte[] image, TuneTable table)
        {
            if (table == null) throw new ArgumentNullException("table");
            if (Flag)
            {
                bool on = System.Math.Abs(table.Values[0][0]) >= 0.5;
                ulong raw = XdfMemory.ReadRaw(image, this, Region, 0, 0);
                raw = on ? (raw | Mask) : (raw & ~Mask);
                XdfMemory.WriteRaw(image, this, Region, 0, 0, raw);
                return;
            }
            if (Transposed)
            {
                for (int row = 0; row < Region.Rows; row++)
                    WriteCell(image, row, 0, table.Values[0][row]);
                return;
            }
            for (int row = 0; row < table.Values.Length; row++)
                for (int col = 0; col < table.Values[row].Length; col++)
                    WriteCell(image, row, col, table.Values[row][col]);
        }

        void WriteCell(byte[] image, int row, int col, double engineering)
        {
            if (HasMin && engineering < Min) engineering = Min;
            if (HasMax && engineering > Max) engineering = Max;
            if (Region.Float)
            {
                double raw = Conversion.InvertFloat(engineering);
                XdfMemory.WriteFloat(image, this, Region, row, col, raw);
                return;
            }
            long rawInteger = Conversion.InvertInteger(engineering, Region.Bits, Region.Signed);
            XdfMemory.WriteRaw(image, this, Region, row, col, unchecked((ulong)rawInteger));
        }

        double[] ReadSeries(byte[] image, XdfSeries series, int count)
        {
            if (count < 1) count = series.Count;
            var labels = new double[count];
            int available = series.Region.Rows * series.Region.Cols;
            for (int i = 0; i < count; i++)
            {
                if (i >= available)
                {
                    labels[i] = i;
                    continue;
                }
                int row = i / series.Region.Cols;
                int col = i % series.Region.Cols;
                labels[i] = XdfMemory.ReadScaled(image, this, series.Region, series.Math, row, col);
            }
            return labels;
        }
    }

    static class XdfMemory
    {
        public static int Locate(byte[] image, long origin, long baseOffset, long baseSubtract, long address)
        {
            if (image == null) throw new ArgumentNullException("image");
            long file = ApplyBase(address, baseOffset, baseSubtract);
            if (file >= 0 && file < image.Length) return (int)file;
            long absolute = address - origin;
            if (absolute >= 0 && absolute < image.Length) return (int)absolute;
            throw new ArgumentOutOfRangeException("address", "Address 0x" + address.ToString("X") + " is outside the ROM.");
        }

        public static long ApplyBase(long address, long baseOffset, long baseSubtract)
        {
            if (baseSubtract == 0) return address + baseOffset;
            if (baseSubtract == 1) return address - baseOffset;
            return address + baseOffset - baseSubtract;
        }

        public static double ReadScaled(byte[] image, XdfBinding binding, XdfRegion region, ScaleEquation math, int row, int col)
        {
            if (region.Float)
            {
                double raw = ReadFloat(image, binding, region, row, col);
                return math.Evaluate(raw);
            }
            ulong bits = ReadRaw(image, binding, region, row, col);
            double value = region.Signed ? Sign(bits, region.Bits) : (double)bits;
            return math.Evaluate(value);
        }

        static double ReadFloat(byte[] image, XdfBinding binding, XdfRegion region, int row, int col)
        {
            ulong bits = ReadRaw(image, binding, region, row, col);
            if (region.Bits == 32)
            {
                byte[] local = new byte[4];
                local[0] = (byte)bits;
                local[1] = (byte)(bits >> 8);
                local[2] = (byte)(bits >> 16);
                local[3] = (byte)(bits >> 24);
                return BitConverter.ToSingle(local, 0);
            }
            return BitConverter.ToDouble(BitConverter.GetBytes(unchecked((long)bits)), 0);
        }

        public static void WriteFloat(byte[] image, XdfBinding binding, XdfRegion region, int row, int col, double raw)
        {
            if (region.Bits == 32)
            {
                byte[] local = BitConverter.GetBytes((float)raw);
                ulong bits = (ulong)(local[0] | (local[1] << 8) | (local[2] << 16) | (local[3] << 24));
                WriteRaw(image, binding, region, row, col, bits);
                return;
            }
            long pattern = BitConverter.ToInt64(BitConverter.GetBytes(raw), 0);
            WriteRaw(image, binding, region, row, col, unchecked((ulong)pattern));
        }

        public static ulong ReadRaw(byte[] image, XdfBinding binding, XdfRegion region, int row, int col)
        {
            long bit = BitPosition(image, binding, region, row, col);
            return ReadBits(image, bit, region.Bits, region.LittleEndian);
        }

        public static void WriteRaw(byte[] image, XdfBinding binding, XdfRegion region, int row, int col, ulong value)
        {
            long bit = BitPosition(image, binding, region, row, col);
            WriteBits(image, bit, region.Bits, region.LittleEndian, value);
        }

        static long BitPosition(byte[] image, XdfBinding binding, XdfRegion region, int row, int col)
        {
            int file = Locate(image, binding.Origin, binding.BaseOffset, binding.BaseSubtract, region.Address);
            return (long)file * 8L + (long)row * region.MajorStride() + (long)col * region.MinorStride();
        }

        public static ulong ReadBits(byte[] image, long bitOffset, int bits, bool littleEndian)
        {
            RequireSpan(image, bitOffset, bits);
            ulong value = 0;
            if (!littleEndian)
            {
                for (int i = 0; i < bits; i++)
                {
                    long bit = bitOffset + i;
                    int mask = 0x80 >> (int)(bit % 8);
                    value <<= 1;
                    if ((image[(int)(bit / 8)] & mask) != 0) value |= 1;
                }
                return value;
            }
            int startByte = (int)(bitOffset / 8);
            int startBit = (int)(bitOffset % 8);
            for (int i = 0; i < bits; i++)
            {
                int global = startBit + i;
                int bit = global % 8;
                if ((image[startByte + global / 8] & (1 << bit)) != 0) value |= 1UL << i;
            }
            return value;
        }

        public static void WriteBits(byte[] image, long bitOffset, int bits, bool littleEndian, ulong value)
        {
            RequireSpan(image, bitOffset, bits);
            if (bits < 64) value &= (1UL << bits) - 1;
            if (!littleEndian)
            {
                for (int i = 0; i < bits; i++)
                {
                    ulong bit = (value >> (bits - 1 - i)) & 1UL;
                    SetBig(image, bitOffset + i, bit == 1);
                }
                return;
            }
            int startByte = (int)(bitOffset / 8);
            int startBit = (int)(bitOffset % 8);
            for (int i = 0; i < bits; i++)
            {
                int global = startBit + i;
                int index = startByte + global / 8;
                int bit = global % 8;
                byte mask = (byte)(1 << bit);
                if ((value & (1UL << i)) != 0) image[index] = (byte)(image[index] | mask);
                else image[index] = (byte)(image[index] & ~mask);
            }
        }

        static void SetBig(byte[] image, long bit, bool on)
        {
            int index = (int)(bit / 8);
            int mask = 0x80 >> (int)(bit % 8);
            if (on) image[index] = (byte)(image[index] | mask);
            else image[index] = (byte)(image[index] & ~mask);
        }

        static void RequireSpan(byte[] image, long bitOffset, int bits)
        {
            if (image == null) throw new ArgumentNullException("image");
            if (bits < 1 || bits > 64) throw new ArgumentOutOfRangeException("bits", "The element size is not supported.");
            if (bitOffset < 0)
                throw new ArgumentOutOfRangeException("bitOffset", "The element starts before the ROM.");
            long end = bitOffset + bits;
            if ((end + 7) / 8 > image.Length)
                throw new ArgumentOutOfRangeException("bitOffset", "The element does not fit in the ROM.");
        }

        static double Sign(ulong raw, int bits)
        {
            ulong sign = 1UL << (bits - 1);
            if ((raw & sign) == 0) return raw;
            return (double)((long)raw - (long)(1UL << bits));
        }
    }

    sealed class ScaleEquation
    {
        public readonly string Source;
        readonly MathNode root;

        ScaleEquation(string source, MathNode root)
        {
            Source = source;
            this.root = root;
        }

        public static ScaleEquation Parse(string equation)
        {
            if (equation == null) equation = "";
            equation = equation.Trim();
            if (equation.Length == 0) equation = "X";
            if (equation.Length > 2000)
                throw new InvalidDataException("The equation is too long.");
            var parser = new MathParser(equation);
            MathNode root = parser.ParseOr();
            parser.End();
            return new ScaleEquation(equation, root);
        }

        public double Evaluate(double x)
        {
            return root.Eval(x);
        }

        public long InvertInteger(double target, int bits, bool signed)
        {
            if (bits < 1 || bits > 32)
                throw new InvalidDataException("The equation cannot be inverted.");
            long min = signed ? -(1L << (bits - 1)) : 0;
            long max = signed ? (1L << (bits - 1)) - 1 : (bits == 32 ? 4294967295L : (1L << bits) - 1);
            double slope;
            double intercept;
            if (TryAffine(out slope, out intercept))
            {
                if (Math.Abs(slope) < 1e-12)
                    throw new InvalidDataException("The equation cannot be inverted.");
                double raw = (target - intercept) / slope;
                if (double.IsNaN(raw) || double.IsInfinity(raw))
                    throw new InvalidDataException("The equation cannot be inverted.");
                long rounded = (long)Math.Round(raw, MidpointRounding.AwayFromZero);
                if (rounded < min) rounded = min;
                if (rounded > max) rounded = max;
                return rounded;
            }
            if (bits <= 16) return Brute(target, min, max);
            bool increasing;
            long low = min;
            long high = max;
            double ignored;
            if (!TryValue(low, out ignored))
            {
                if (low >= high || !TryValue(low + 1, out ignored))
                    throw new InvalidDataException("The equation cannot be inverted.");
                low++;
            }
            if (!TryValue(high, out ignored))
            {
                if (high <= low || !TryValue(high - 1, out ignored))
                    throw new InvalidDataException("The equation cannot be inverted.");
                high--;
            }
            if (!Monotonic(low, high, out increasing))
                throw new InvalidDataException("The equation cannot be inverted.");
            return Search(target, low, high, increasing);
        }

        public double InvertFloat(double target)
        {
            double slope;
            double intercept;
            if (TryAffine(out slope, out intercept))
            {
                if (Math.Abs(slope) < 1e-12)
                    throw new InvalidDataException("The equation cannot be inverted.");
                double raw = (target - intercept) / slope;
                if (double.IsNaN(raw) || double.IsInfinity(raw))
                    throw new InvalidDataException("The equation cannot be inverted.");
                return raw;
            }
            bool increasing = false;
            double previous = 0;
            bool have = false;
            int direction = 0;
            int valid = 0;
            for (int i = 0; i <= 32; i++)
            {
                double sample = -1e6 + (2e6 * i / 32.0);
                double y;
                if (!TryValue(sample, out y)) continue;
                valid++;
                if (have)
                {
                    double delta = y - previous;
                    double scale = Math.Max(1.0, Math.Max(Math.Abs(y), Math.Abs(previous)));
                    if (delta > 1e-7 * scale)
                    {
                        if (direction < 0) throw new InvalidDataException("The equation cannot be inverted.");
                        direction = 1;
                    }
                    else if (delta < -1e-7 * scale)
                    {
                        if (direction > 0) throw new InvalidDataException("The equation cannot be inverted.");
                        direction = -1;
                    }
                }
                previous = y;
                have = true;
            }
            if (valid < 2) throw new InvalidDataException("The equation cannot be inverted.");
            increasing = direction >= 0;
            double low = -1e6;
            double high = 1e6;
            for (int i = 0; i < 60; i++)
            {
                double mid = (low + high) / 2.0;
                double y;
                if (!TryValue(mid, out y))
                    throw new InvalidDataException("The equation cannot be inverted.");
                if (increasing)
                {
                    if (y < target) low = mid;
                    else high = mid;
                }
                else if (y > target) low = mid;
                else high = mid;
            }
            return (low + high) / 2.0;
        }

        bool TryAffine(out double slope, out double intercept)
        {
            slope = 0;
            intercept = 0;
            double f0;
            double f1;
            if (!TryValue(0, out f0) || !TryValue(1, out f1)) return false;
            slope = f1 - f0;
            intercept = f0;
            double[] probes = new double[] { 0, 1, 2, -1, 100 };
            for (int i = 0; i < probes.Length; i++)
            {
                double actual;
                if (!TryValue(probes[i], out actual)) return false;
                double expected = f0 + slope * probes[i];
                double scale = 1.0 + Math.Abs(expected);
                if (Math.Abs(actual - expected) > 1e-6 * scale) return false;
            }
            return true;
        }

        bool TryValue(double x, out double y)
        {
            y = Evaluate(x);
            return !double.IsNaN(y) && !double.IsInfinity(y);
        }

        long Brute(double target, long min, long max)
        {
            bool any = false;
            long best = min;
            double bestDistance = 0;
            for (long raw = min; raw <= max; raw++)
            {
                double y;
                if (!TryValue(raw, out y)) continue;
                double distance = Math.Abs(y - target);
                if (!any || distance < bestDistance - 1e-9 || (Math.Abs(distance - bestDistance) <= 1e-9 && Math.Abs(raw) < Math.Abs(best)))
                {
                    any = true;
                    best = raw;
                    bestDistance = distance;
                }
            }
            if (!any) throw new InvalidDataException("The equation cannot be inverted.");
            return best;
        }

        bool Monotonic(long low, long high, out bool increasing)
        {
            increasing = true;
            int direction = 0;
            bool have = false;
            double previous = 0;
            int valid = 0;
            int samples = 48;
            for (int i = 0; i <= samples; i++)
            {
                long raw = low + (high - low) * i / samples;
                double y;
                if (!TryValue(raw, out y)) continue;
                valid++;
                if (have)
                {
                    double delta = y - previous;
                    double scale = Math.Max(1.0, Math.Max(Math.Abs(y), Math.Abs(previous)));
                    if (delta > 1e-6 * scale)
                    {
                        if (direction < 0) return false;
                        direction = 1;
                    }
                    else if (delta < -1e-6 * scale)
                    {
                        if (direction > 0) return false;
                        direction = -1;
                    }
                }
                previous = y;
                have = true;
            }
            if (valid < 2) return false;
            increasing = direction >= 0;
            return true;
        }

        long Search(double target, long low, long high, bool increasing)
        {
            return Search(target, low, high, increasing, low, high);
        }

        long Search(double target, long low, long high, bool increasing, long min, long max)
        {
            long best = low;
            double bestDistance = double.MaxValue;
            Consider(target, low, min, max, ref best, ref bestDistance);
            while (low < high)
            {
                long mid = low + (high - low) / 2;
                double y;
                if (!TryValue(mid, out y))
                    throw new InvalidDataException("The equation cannot be inverted.");
                Consider(target, mid, min, max, ref best, ref bestDistance);
                if (increasing)
                {
                    if (y < target) low = mid + 1;
                    else high = mid;
                }
                else if (y > target) low = mid + 1;
                else high = mid;
            }
            Consider(target, low, min, max, ref best, ref bestDistance);
            Consider(target, low - 1, min, max, ref best, ref bestDistance);
            Consider(target, low + 1, min, max, ref best, ref bestDistance);
            return best;
        }

        void Consider(double target, long raw, long min, long max, ref long best, ref double bestDistance)
        {
            double y;
            if (raw < min || raw > max) return;
            if (!TryValue(raw, out y)) return;
            double distance = Math.Abs(y - target);
            if (distance < bestDistance)
            {
                bestDistance = distance;
                best = raw;
            }
        }
    }

    abstract class MathNode
    {
        public abstract double Eval(double x);
    }

    sealed class NumberNode : MathNode
    {
        readonly double value;
        public NumberNode(double value) { this.value = value; }
        public override double Eval(double x) { return value; }
    }

    sealed class VariableNode : MathNode
    {
        public override double Eval(double x) { return x; }
    }

    sealed class UnaryNode : MathNode
    {
        readonly MathNode inner;
        public UnaryNode(MathNode inner) { this.inner = inner; }
        public override double Eval(double x) { return -inner.Eval(x); }
    }

    sealed class BinaryNode : MathNode
    {
        readonly char op;
        readonly MathNode left;
        readonly MathNode right;

        public BinaryNode(char op, MathNode left, MathNode right)
        {
            this.op = op;
            this.left = left;
            this.right = right;
        }

        public override double Eval(double x)
        {
            double a = left.Eval(x);
            double b = right.Eval(x);
            if (op == '+') return a + b;
            if (op == '-') return a - b;
            if (op == '*') return a * b;
            if (op == '/')
            {
                if (b == 0) return double.NaN;
                return a / b;
            }
            if (op == '%')
            {
                if (b == 0) return double.NaN;
                return a % b;
            }
            if (double.IsNaN(a) || double.IsNaN(b) || double.IsInfinity(a) || double.IsInfinity(b)) return double.NaN;
            if (Math.Abs(a) > 9.22e18 || Math.Abs(b) > 9.22e18) return double.NaN;
            long ia = (long)Math.Round(a, MidpointRounding.AwayFromZero);
            long ib = (long)Math.Round(b, MidpointRounding.AwayFromZero);
            if (op == '&') return ia & ib;
            if (op == '|') return ia | ib;
            return ia ^ ib;
        }
    }

    sealed class MathParser
    {
        readonly string text;
        int index;
        int depth;

        public MathParser(string text)
        {
            this.text = text;
        }

        public MathNode ParseOr()
        {
            MathNode left = ParseXor();
            while (Take('|')) left = new BinaryNode('|', left, ParseXor());
            return left;
        }

        public void End()
        {
            Skip();
            if (index < text.Length)
                throw new InvalidDataException("The equation has an unexpected character.");
        }

        MathNode ParseXor()
        {
            MathNode left = ParseAnd();
            while (Take('^')) left = new BinaryNode('^', left, ParseAnd());
            return left;
        }

        MathNode ParseAnd()
        {
            MathNode left = ParseAdd();
            while (Take('&')) left = new BinaryNode('&', left, ParseAdd());
            return left;
        }

        MathNode ParseAdd()
        {
            MathNode left = ParseMul();
            while (true)
            {
                if (Take('+')) left = new BinaryNode('+', left, ParseMul());
                else if (Take('-')) left = new BinaryNode('-', left, ParseMul());
                else break;
            }
            return left;
        }

        MathNode ParseMul()
        {
            MathNode left = ParseUnary();
            while (true)
            {
                if (Take('*')) left = new BinaryNode('*', left, ParseUnary());
                else if (Take('/')) left = new BinaryNode('/', left, ParseUnary());
                else if (Take('%')) left = new BinaryNode('%', left, ParseUnary());
                else break;
            }
            return left;
        }

        MathNode ParseUnary()
        {
            if (Take('+')) return ParseUnary();
            if (Take('-')) return new UnaryNode(ParseUnary());
            return ParsePrimary();
        }

        MathNode ParsePrimary()
        {
            Skip();
            if (index >= text.Length)
                throw new InvalidDataException("The equation ended early.");
            if (Take('('))
            {
                depth++;
                if (depth > 64) throw new InvalidDataException("The equation is too deeply nested.");
                MathNode inner = ParseOr();
                if (!Take(')')) throw new InvalidDataException("The equation is missing a closing parenthesis.");
                depth--;
                return inner;
            }
            char c = text[index];
            if ((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z'))
            {
                int start = index;
                index++;
                while (index < text.Length)
                {
                    char n = text[index];
                    if ((n >= 'A' && n <= 'Z') || (n >= 'a' && n <= 'z') || (n >= '0' && n <= '9')) index++;
                    else break;
                }
                string name = text.Substring(start, index - start);
                if (name == "X" || name == "x") return new VariableNode();
                throw new InvalidDataException("The equation uses an unknown name.");
            }
            if ((c >= '0' && c <= '9') || c == '.') return new NumberNode(ReadNumber());
            throw new InvalidDataException("The equation has an unexpected character.");
        }

        double ReadNumber()
        {
            int start = index;
            if (text.Length - index > 2 && text[index] == '0' && (text[index + 1] == 'x' || text[index + 1] == 'X'))
            {
                index += 2;
                int hexStart = index;
                while (index < text.Length && IsHex(text[index])) index++;
                if (index == hexStart) throw new InvalidDataException("The equation has a bad number.");
                return Convert.ToInt64(text.Substring(hexStart, index - hexStart), 16);
            }
            bool dot = false;
            while (index < text.Length)
            {
                char c = text[index];
                if (c >= '0' && c <= '9') index++;
                else if (c == '.' && !dot)
                {
                    dot = true;
                    index++;
                }
                else break;
            }
            double value;
            if (!double.TryParse(text.Substring(start, index - start), NumberStyles.Float, CultureInfo.InvariantCulture, out value))
                throw new InvalidDataException("The equation has a bad number.");
            return value;
        }

        static bool IsHex(char c)
        {
            return (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F');
        }

        bool Take(char expected)
        {
            Skip();
            if (index < text.Length && text[index] == expected)
            {
                index++;
                return true;
            }
            return false;
        }

        void Skip()
        {
            while (index < text.Length && char.IsWhiteSpace(text[index])) index++;
        }
    }

    sealed class XdfLabel
    {
        public int Index;
        public string Text = "";
    }

    sealed class XdfAxis
    {
        public string UniqueId = "";
        public XdfRegion Region;
        public ScaleEquation Math;
        public readonly List<XdfLabel> Labels = new List<XdfLabel>();
        public string LinkId = "";
        public string LinkType = "";
        public string Units = "";
        public int Decimals = -1;
        public int IndexCount;
        public bool HasMin;
        public bool HasMax;
        public double Min;
        public double Max;
    }

    sealed class XdfItem
    {
        public string Id = "";
        public string Title = "";
        public string Category = "Uncategorized";
        public bool Flag;
        public ulong Mask;
        public string Error = "";
        public XdfAxis X;
        public XdfAxis Y;
        public XdfAxis Z;
    }

    sealed class XdfLink
    {
        public XdfAxis Axis;
        public XdfItem Item;
        public bool Ambiguous;
    }

    sealed class XdfBindResult
    {
        public TuneDocument Document = new TuneDocument();
        public readonly List<string> Errors = new List<string>();
    }

    sealed class XdfDocument
    {
        public string Title = "";
        public string Description = "";
        public long BaseOffset;
        public long BaseSubtract;
        public int DefaultBits = 8;
        public int DefaultDigits = -1;
        public bool DefaultSigned;
        public bool DefaultLittle;
        public bool DefaultFloat;
        public readonly Dictionary<long, string> Categories = new Dictionary<long, string>();
        public readonly List<XdfItem> Items = new List<XdfItem>();

        public static XdfDocument Load(string path)
        {
            if (path == null || path.Trim().Length == 0) throw new ArgumentException("The definition path is empty.");
            if (path.EndsWith(".enc", StringComparison.OrdinalIgnoreCase))
                throw new InvalidDataException("Encrypted definition files are not opened.");
            using (XmlReader reader = XmlReader.Create(path, ReaderSettings()))
                return Parse(reader);
        }

        public static XdfDocument Parse(string xml)
        {
            if (xml == null || xml.Trim().Length == 0)
                throw new InvalidDataException("The definition is empty.");
            using (XmlReader reader = XmlReader.Create(new StringReader(xml), ReaderSettings()))
                return Parse(reader);
        }

        public XdfBindResult Bind(byte[] image, long origin)
        {
            if (image == null) throw new ArgumentNullException("image");
            var result = new XdfBindResult();
            result.Document.Persist = false;
            result.Document.Note = Title.Length == 0 ? "ROM calibration." : Title;
            var links = IndexLinks();
            var used = new Dictionary<string, int>();
            for (int i = 0; i < Items.Count; i++)
            {
                XdfItem item = Items[i];
                if (item.Error.Length > 0)
                {
                    result.Errors.Add(item.Error);
                    continue;
                }
                try
                {
                    TuneTable table = Build(item, image, origin, links);
                    string id = item.Id.Length == 0 ? "item-" + (i + 1).ToString(CultureInfo.InvariantCulture) : item.Id;
                    int seen;
                    if (used.TryGetValue(id, out seen))
                    {
                        seen++;
                        used[id] = seen;
                        id = id + "-" + seen.ToString(CultureInfo.InvariantCulture);
                    }
                    else used[id] = 1;
                    table.Id = id;
                    result.Document.Tables.Add(table);
                }
                catch (Exception ex)
                {
                    result.Errors.Add(item.Title + ": " + ex.Message);
                }
            }
            return result;
        }

        TuneTable Build(XdfItem item, byte[] image, long origin, Dictionary<string, XdfLink> links)
        {
            if (item.Z == null || item.Z.Region == null || !item.Z.Region.HasAddress)
                throw new InvalidDataException("The value has no address.");
            XdfRegion region = item.Z.Region;
            Normalize(item, region);
            if (region.Rows < 1 || region.Cols < 1)
                throw new InvalidDataException("The table shape is not valid.");
            if ((long)region.Rows * region.Cols > 262144)
                throw new InvalidDataException("The table has too many cells.");
            var binding = new XdfBinding();
            binding.Conversion = item.Z.Math == null ? ScaleEquation.Parse("X") : item.Z.Math;
            binding.Region = region;
            binding.Flag = item.Flag;
            binding.Mask = item.Mask;
            binding.Origin = origin;
            binding.BaseOffset = BaseOffset;
            binding.BaseSubtract = BaseSubtract;
            binding.HasMin = item.Z.HasMin;
            binding.HasMax = item.Z.HasMax;
            binding.Min = item.Z.Min;
            binding.Max = item.Z.Max;
            if (item.Flag)
            {
                int bits = region.Bits;
                ulong width = bits >= 64 ? ulong.MaxValue : (1UL << bits) - 1;
                if ((item.Mask & width) == 0) throw new InvalidDataException("The flag mask is zero.");
                binding.Mask = item.Mask & width;
            }
            var stack = new List<string>();
            int xCount = region.Cols;
            int yCount = region.Rows;
            bool transposed = !item.Flag && region.Rows > 1 && region.Cols == 1;
            if (transposed)
            {
                binding.Transposed = true;
                xCount = region.Rows;
                yCount = 1;
            }
            XdfSeries xSeries = ResolveAxis(item.X, xCount, item, image, binding, links, stack);
            XdfSeries ySeries = transposed || item.Flag || (region.Rows == 1 && region.Cols >= 1)
                ? FixedSeries(new double[] { 0 })
                : ResolveAxis(item.Y, yCount, item, image, binding, links, stack);
            var table = new TuneTable();
            table.Name = item.Title;
            table.Domain = item.Category;
            table.Level = "beginner";
            table.Unit = item.Z.Units.Length == 0 ? (item.Flag ? "on/off" : "") : item.Z.Units;
            table.Decimals = item.Z.Decimals;
            table.Binding = binding;
            binding.AxisX = xSeries;
            binding.AxisY = ySeries;
            if (item.Flag || (region.Rows == 1 && region.Cols == 1))
            {
                table.Kind = "scalar";
                table.XName = "Value";
                table.YName = "Value";
                table.X = new double[] { 0 };
                table.Y = new double[] { 0 };
                table.Values = new double[][] { new double[] { 0 } };
            }
            else if (region.Rows == 1 || transposed)
            {
                table.Kind = "curve";
                table.XName = AxisName(transposed ? item.Y : item.X, "X");
                table.YName = table.Unit;
                table.X = LabelsOf(image, binding, xSeries, xCount);
                table.Y = new double[] { 0 };
                table.Values = new double[1][];
                table.Values[0] = new double[xCount];
            }
            else
            {
                table.Kind = "map";
                table.XName = AxisName(item.X, "X");
                table.YName = AxisName(item.Y, "Y");
                table.X = LabelsOf(image, binding, xSeries, region.Cols);
                table.Y = LabelsOf(image, binding, ySeries, region.Rows);
                table.Values = new double[region.Rows][];
                for (int row = 0; row < region.Rows; row++) table.Values[row] = new double[region.Cols];
            }
            binding.ReadInto(image, table);
            table.Base = TuneDocument.Clone(table.Values);
            return table;
        }

        static void Normalize(XdfItem item, XdfRegion region)
        {
            if (!region.ColsSpecified)
            {
                if (item.X != null && item.X.IndexCount > 0) region.Cols = item.X.IndexCount;
                else if (item.X != null && item.X.Labels.Count > 0) region.Cols = item.X.Labels.Count;
                else region.Cols = 1;
            }
            if (!region.RowsSpecified)
            {
                if (item.Y != null && item.Y.IndexCount > 0) region.Rows = item.Y.IndexCount;
                else if (item.Y != null && item.Y.Labels.Count > 0) region.Rows = item.Y.Labels.Count;
                else region.Rows = 1;
            }
            if (item.Flag)
            {
                region.Rows = 1;
                region.Cols = 1;
            }
        }

        static string AxisName(XdfAxis axis, string fallback)
        {
            if (axis != null && axis.Units.Length > 0) return axis.Units;
            return fallback;
        }

        static double[] LabelsOf(byte[] image, XdfBinding binding, XdfSeries series, int count)
        {
            if (series.Fixed != null)
            {
                var copy = new double[count];
                for (int i = 0; i < count; i++) copy[i] = i < series.Fixed.Length ? series.Fixed[i] : i;
                return copy;
            }
            var labels = new double[count];
            int available = series.Region.Rows * series.Region.Cols;
            for (int i = 0; i < count; i++)
            {
                if (i >= available)
                {
                    labels[i] = i;
                    continue;
                }
                int row = i / series.Region.Cols;
                int col = i % series.Region.Cols;
                labels[i] = XdfMemory.ReadScaled(image, binding, series.Region, series.Math, row, col);
            }
            return labels;
        }

        XdfSeries ResolveAxis(XdfAxis axis, int needed, XdfItem owner, byte[] image, XdfBinding binding, Dictionary<string, XdfLink> links, List<string> stack)
        {
            if (axis == null) return FixedSeries(Indexes(needed));
            if (axis.LinkId.Length > 0)
            {
                double[] linked = ResolveLink(axis.LinkId, axis.LinkType, owner, image, binding, links, stack);
                if (linked == null || linked.Length == 0)
                    throw new InvalidDataException("Linked axis " + axis.LinkId + " was not found for " + owner.Title + ".");
                return FixedSeries(Fit(linked, needed));
            }
            if (axis.Region != null && axis.Region.HasAddress)
            {
                PrepareAxisRegion(axis);
                var series = new XdfSeries();
                series.Region = axis.Region;
                series.Math = axis.Math == null ? ScaleEquation.Parse("X") : axis.Math;
                series.Count = axis.Region.Rows * axis.Region.Cols;
                return series;
            }
            if (axis.Labels.Count > 0) return FixedSeries(Fit(LabelValues(axis), needed));
            return FixedSeries(Indexes(needed));
        }

        static void PrepareAxisRegion(XdfAxis axis)
        {
            XdfRegion region = axis.Region;
            int count = axis.IndexCount;
            if (count < 1)
            {
                if (region.RowsSpecified || region.ColsSpecified) count = region.Rows * region.Cols;
                else count = 1;
            }
            if (!region.RowsSpecified && !region.ColsSpecified)
            {
                region.Rows = 1;
                region.Cols = count < 1 ? 1 : count;
            }
            else if (!region.RowsSpecified) region.Rows = 1;
            else if (!region.ColsSpecified) region.Cols = 1;
            if (region.Rows < 1) region.Rows = 1;
            if (region.Cols < 1) region.Cols = 1;
        }

        double[] ResolveLink(string id, string type, XdfItem owner, byte[] image, XdfBinding binding, Dictionary<string, XdfLink> links, List<string> stack)
        {
            for (int i = 0; i < stack.Count; i++)
                if (stack[i] == id)
                    throw new InvalidDataException("Linked axis " + id + " cycles for " + owner.Title + ".");
            XdfLink link;
            if (!links.TryGetValue(id, out link) || link == null)
                throw new InvalidDataException("Linked axis " + id + " was not found for " + owner.Title + ".");
            if (link.Ambiguous)
                throw new InvalidDataException("Linked axis " + id + " is not unique for " + owner.Title + ".");
            stack.Add(id);
            double[] values;
            try
            {
                if (link.Axis != null) values = ValuesOfAxis(link.Axis, owner, image, binding, links, stack);
                else if (link.Item != null) values = ValuesOfItem(link.Item, type, owner, image, binding, links, stack);
                else values = null;
            }
            finally
            {
                stack.RemoveAt(stack.Count - 1);
            }
            return values;
        }

        double[] ValuesOfAxis(XdfAxis axis, XdfItem owner, byte[] image, XdfBinding binding, Dictionary<string, XdfLink> links, List<string> stack)
        {
            if (axis.LinkId.Length > 0) return ResolveLink(axis.LinkId, axis.LinkType, owner, image, binding, links, stack);
            if (axis.Region != null && axis.Region.HasAddress)
            {
                PrepareAxisRegion(axis);
                return ReadRegion(image, binding, axis.Region, axis.Math);
            }
            if (axis.Labels.Count > 0) return LabelValues(axis);
            return null;
        }

        double[] ValuesOfItem(XdfItem item, string type, XdfItem owner, byte[] image, XdfBinding binding, Dictionary<string, XdfLink> links, List<string> stack)
        {
            string kind = type == null ? "" : type.Trim().ToLowerInvariant();
            XdfAxis axis;
            if (kind == "x" || kind == "0") axis = item.X;
            else if (kind == "y" || kind == "1") axis = item.Y;
            else axis = item.Z;
            if (axis == null) return null;
            return ValuesOfAxis(axis, owner, image, binding, links, stack);
        }

        static double[] ReadRegion(byte[] image, XdfBinding binding, XdfRegion region, ScaleEquation math)
        {
            if (math == null) math = ScaleEquation.Parse("X");
            int count = region.Rows * region.Cols;
            var values = new double[count];
            int index = 0;
            for (int row = 0; row < region.Rows; row++)
            {
                for (int col = 0; col < region.Cols; col++)
                {
                    values[index] = XdfMemory.ReadScaled(image, binding, region, math, row, col);
                    index++;
                }
            }
            return values;
        }

        static double[] LabelValues(XdfAxis axis)
        {
            var ordered = new List<XdfLabel>(axis.Labels);
            ordered.Sort(delegate (XdfLabel a, XdfLabel b) { return a.Index.CompareTo(b.Index); });
            var values = new double[ordered.Count];
            for (int i = 0; i < ordered.Count; i++)
            {
                double parsed;
                if (ordered[i].Text != null && double.TryParse(ordered[i].Text.Trim(), NumberStyles.Float, CultureInfo.InvariantCulture, out parsed))
                    values[i] = parsed;
                else values[i] = ordered[i].Index;
            }
            return values;
        }

        static double[] Fit(double[] source, int needed)
        {
            var values = new double[needed];
            for (int i = 0; i < needed; i++) values[i] = i < source.Length ? source[i] : i;
            return values;
        }

        static double[] Indexes(int count)
        {
            var values = new double[count];
            for (int i = 0; i < count; i++) values[i] = i;
            return values;
        }

        static XdfSeries FixedSeries(double[] values)
        {
            var series = new XdfSeries();
            series.Fixed = values;
            series.Count = values.Length;
            series.Math = ScaleEquation.Parse("X");
            return series;
        }

        Dictionary<string, XdfLink> IndexLinks()
        {
            var links = new Dictionary<string, XdfLink>();
            for (int i = 0; i < Items.Count; i++)
            {
                AddLink(links, Items[i].Id, null, Items[i]);
                AddAxisLink(links, Items[i].X);
                AddAxisLink(links, Items[i].Y);
                AddAxisLink(links, Items[i].Z);
            }
            return links;
        }

        static void AddAxisLink(Dictionary<string, XdfLink> links, XdfAxis axis)
        {
            if (axis == null) return;
            AddLink(links, axis.UniqueId, axis, null);
        }

        static void AddLink(Dictionary<string, XdfLink> links, string id, XdfAxis axis, XdfItem item)
        {
            if (id == null || id.Length == 0) return;
            XdfLink existing;
            if (links.TryGetValue(id, out existing))
            {
                existing.Ambiguous = true;
                existing.Axis = null;
                existing.Item = null;
                return;
            }
            var link = new XdfLink();
            link.Axis = axis;
            link.Item = item;
            links[id] = link;
        }

        static XdfDocument Parse(XmlReader reader)
        {
            var xml = new XmlDocument();
            xml.XmlResolver = null;
            xml.Load(reader);
            XmlElement root = xml.DocumentElement;
            if (root == null) throw new InvalidDataException("The definition is empty.");
            var document = new XdfDocument();
            foreach (XmlNode node in root.ChildNodes)
            {
                XmlElement element = node as XmlElement;
                if (element == null) continue;
                if (NameIs(element, "XDFHEADER") && document.Title.Length == 0 && document.Categories.Count == 0)
                    document.ReadHeader(element);
            }
            foreach (XmlNode node in root.ChildNodes)
            {
                XmlElement element = node as XmlElement;
                if (element == null) continue;
                if (NameIs(element, "XDFTABLE") || NameIs(element, "XDFCONSTANT") || NameIs(element, "XDFFLAG"))
                    document.Items.Add(document.ReadItem(element));
            }
            return document;
        }

        void ReadHeader(XmlElement header)
        {
            string title = ChildText(header, "deftitle");
            if (title == null) title = ChildText(header, "title");
            Title = title == null ? "" : title;
            string description = ChildText(header, "description");
            Description = description == null ? "" : description;
            foreach (XmlElement offset in Children(header, "BASEOFFSET"))
            {
                ReadBaseOffset(offset);
                break;
            }
            foreach (XmlElement defaults in Children(header, "DEFAULTS"))
            {
                ReadDefaults(defaults);
                break;
            }
            foreach (XmlElement category in Children(header, "CATEGORY"))
            {
                string indexText = Attr(category, "index");
                string name = Attr(category, "name");
                if (name == null || name.Trim().Length == 0) name = category.InnerText;
                if (indexText == null || name == null || name.Trim().Length == 0) continue;
                long index;
                try { index = ParseInteger(indexText); }
                catch (FormatException) { continue; }
                if (!Categories.ContainsKey(index)) Categories[index] = name.Trim();
            }
        }

        void ReadBaseOffset(XmlElement element)
        {
            string offsetText = Attr(element, "offset");
            if (offsetText == null || offsetText.Trim().Length == 0) offsetText = element.InnerText;
            string subtractText = Attr(element, "subtract");
            if (offsetText != null && offsetText.Trim().Length > 0) BaseOffset = ParseInteger(offsetText);
            if (subtractText != null && subtractText.Trim().Length > 0) BaseSubtract = ParseInteger(subtractText);
        }

        void ReadDefaults(XmlElement element)
        {
            string bits = Attr(element, "datasizeinbits");
            string digits = Attr(element, "sigdigits");
            if (bits != null && bits.Trim().Length > 0) DefaultBits = (int)ParseInteger(bits);
            if (digits != null && digits.Trim().Length > 0) DefaultDigits = (int)ParseInteger(digits);
            DefaultSigned = FlagValue(Attr(element, "signed"));
            DefaultLittle = FlagValue(Attr(element, "lsbfirst"));
            DefaultFloat = FlagValue(Attr(element, "float"));
            if (DefaultBits < 1) DefaultBits = 8;
        }

        XdfItem ReadItem(XmlElement element)
        {
            var item = new XdfItem();
            bool flag = NameIs(element, "XDFFLAG");
            item.Flag = flag;
            item.Title = ChildText(element, "title");
            if (item.Title == null || item.Title.Length == 0) item.Title = "Untitled";
            item.Id = CanonicalId(Attr(element, "uniqueid"));
            item.Category = ResolveCategory(element, flag);
            try
            {
                if (flag || NameIs(element, "XDFCONSTANT"))
                {
                    item.Z = new XdfAxis();
                    item.Z.Math = ReadMath(element);
                    item.Z.Units = TextOrEmpty(ChildText(element, "units"));
                    item.Z.Decimals = ReadDigits(element);
                    ReadLimit(element, item.Z);
                    XmlElement data = FirstChild(element, "EMBEDDEDDATA");
                    if (data == null) throw new InvalidDataException(item.Title + " has no address.");
                    item.Z.Region = ReadRegion(data, true);
                    if (flag) item.Mask = ReadMask(element);
                }
                else
                {
                    foreach (XmlElement axisElement in Children(element, "XDFAXIS"))
                    {
                        XdfAxis axis = ReadAxis(axisElement);
                        string id = Attr(axisElement, "id");
                        if (id == null) continue;
                        if (string.Equals(id, "x", StringComparison.OrdinalIgnoreCase)) item.X = axis;
                        else if (string.Equals(id, "y", StringComparison.OrdinalIgnoreCase)) item.Y = axis;
                        else if (string.Equals(id, "z", StringComparison.OrdinalIgnoreCase)) item.Z = axis;
                    }
                    if (item.Z == null) throw new InvalidDataException(item.Title + " has no value axis.");
                    if (item.Z.Region == null || !item.Z.Region.HasAddress)
                        throw new InvalidDataException(item.Title + " has no address.");
                }
            }
            catch (Exception ex)
            {
                item.Error = ex.Message;
            }
            return item;
        }

        XdfAxis ReadAxis(XmlElement element)
        {
            var axis = new XdfAxis();
            axis.UniqueId = CanonicalId(Attr(element, "uniqueid"));
            axis.Units = TextOrEmpty(ChildText(element, "units"));
            axis.Decimals = ReadDigits(element);
            ReadLimit(element, axis);
            int indexCount;
            if (TryChildInt(element, "indexcount", out indexCount)) axis.IndexCount = indexCount;
            axis.Math = ReadMath(element);
            foreach (XmlElement info in Children(element, "embedinfo"))
            {
                string link = Attr(info, "linkobjid");
                if (link == null || link.Trim().Length == 0) link = Attr(info, "uniqueid");
                if (link != null && link.Trim().Length > 0)
                {
                    axis.LinkId = CanonicalId(link);
                    string type = Attr(info, "type");
                    axis.LinkType = type == null ? "" : type.Trim();
                }
            }
            foreach (XmlElement label in Children(element, "LABEL"))
            {
                var entry = new XdfLabel();
                entry.Text = Attr(label, "value");
                if (entry.Text == null) entry.Text = label.InnerText == null ? "" : label.InnerText.Trim();
                string indexText = Attr(label, "index");
                entry.Index = axis.Labels.Count;
                if (indexText != null && indexText.Trim().Length > 0) entry.Index = (int)ParseInteger(indexText);
                axis.Labels.Add(entry);
            }
            XmlElement data = FirstChild(element, "EMBEDDEDDATA");
            if (data != null)
            {
                string address = Attr(data, "mmedaddress");
                if (address == null || address.Trim().Length == 0) address = ChildText(data, "mmedaddress");
                if (address != null && address.Trim().Length > 0) axis.Region = ReadRegion(data, true);
            }
            return axis;
        }

        XdfRegion ReadRegion(XmlElement data, bool requireAddress)
        {
            string addressText = Attr(data, "mmedaddress");
            if (addressText == null || addressText.Trim().Length == 0) addressText = ChildText(data, "mmedaddress");
            if (addressText == null || addressText.Trim().Length == 0) addressText = Attr(data, "address");
            var region = new XdfRegion();
            if (addressText != null && addressText.Trim().Length > 0)
            {
                region.Address = ParseInteger(addressText);
                region.HasAddress = true;
            }
            else if (requireAddress && Attr(data, "mmedelementsizebits") == null)
            {
                throw new InvalidDataException("A value needs an address.");
            }
            string flagsText = Attr(data, "mmedtypeflags");
            if (flagsText == null || flagsText.Trim().Length == 0)
            {
                region.Signed = DefaultSigned;
                region.LittleEndian = DefaultLittle;
                region.Float = DefaultFloat;
            }
            else
            {
                long flags = ParseInteger(flagsText);
                region.Signed = (flags & 0x01) != 0;
                region.LittleEndian = (flags & 0x02) != 0;
                region.Float = (flags & 0x10000) != 0;
            }
            string sizeText = Attr(data, "mmedelementsizebits");
            region.Bits = sizeText == null || sizeText.Trim().Length == 0 ? DefaultBits : (int)ParseInteger(sizeText);
            if (region.Bits < 1 || region.Bits > 64)
                throw new InvalidDataException("The element size is not supported.");
            if (region.Float && region.Bits != 32 && region.Bits != 64)
                throw new InvalidDataException("A floating-point value must be 32 or 64 bits.");
            if (!region.Float && region.Bits > 32)
                throw new InvalidDataException("An integer value must be 32 bits or smaller.");
            region.MajorBits = OptionalIntAttr(data, "mmedmajorstridebits");
            region.MinorBits = OptionalIntAttr(data, "mmedminorstridebits");
            string rowText = Attr(data, "mmedrowcount");
            string colText = Attr(data, "mmedcolcount");
            if (rowText != null && rowText.Trim().Length > 0)
            {
                region.Rows = (int)ParseInteger(rowText);
                region.RowsSpecified = true;
            }
            if (colText != null && colText.Trim().Length > 0)
            {
                region.Cols = (int)ParseInteger(colText);
                region.ColsSpecified = true;
            }
            if (region.RowsSpecified && region.Rows < 1) throw new InvalidDataException("The table shape is not valid.");
            if (region.ColsSpecified && region.Cols < 1) throw new InvalidDataException("The table shape is not valid.");
            if (!region.RowsSpecified) region.Rows = 1;
            if (!region.ColsSpecified) region.Cols = 1;
            if (!region.HasAddress && requireAddress)
                throw new InvalidDataException("A value needs an address.");
            return region;
        }

        int ReadDigits(XmlElement element)
        {
            int digits;
            if (TryChildInt(element, "decimalpl", out digits)) return digits;
            string attr = Attr(element, "decimalpl");
            if (attr != null && attr.Trim().Length > 0) return (int)ParseInteger(attr);
            return DefaultDigits;
        }

        static void ReadLimit(XmlElement element, XdfAxis axis)
        {
            double min;
            double max;
            if (TryChildDouble(element, "min", out min))
            {
                axis.HasMin = true;
                axis.Min = min;
            }
            if (TryChildDouble(element, "max", out max))
            {
                axis.HasMax = true;
                axis.Max = max;
            }
        }

        static ulong ReadMask(XmlElement element)
        {
            string text = ChildText(element, "mask");
            if (text == null || text.Trim().Length == 0) text = Attr(element, "mask");
            if (text == null || text.Trim().Length == 0)
                throw new InvalidDataException("A flag needs a mask.");
            return ParseUnsigned(text);
        }

        string ResolveCategory(XmlElement item, bool flag)
        {
            foreach (XmlElement mem in Children(item, "CATEGORYMEM"))
            {
                string raw = Attr(mem, "category");
                if (raw == null || raw.Trim().Length == 0) raw = Attr(mem, "index");
                if (raw == null || raw.Trim().Length == 0) continue;
                long id;
                try { id = ParseInteger(raw); }
                catch (FormatException) { continue; }
                string name;
                if (Categories.TryGetValue(id, out name)) return name;
                if (Categories.TryGetValue(id - 1, out name)) return name;
            }
            return flag ? "Flags" : "Uncategorized";
        }

        static ScaleEquation ReadMath(XmlElement parent)
        {
            foreach (XmlElement math in Children(parent, "MATH"))
            {
                string equation = Attr(math, "equation");
                if (equation == null || equation.Trim().Length == 0)
                {
                    equation = ChildText(math, "EQUATION");
                    if (equation == null || equation.Trim().Length == 0)
                    {
                        string inner = math.InnerText == null ? "" : math.InnerText.Trim();
                        if (inner.Length > 0) equation = inner;
                    }
                }
                if (equation != null && equation.Trim().Length > 0) return ScaleEquation.Parse(equation);
            }
            return ScaleEquation.Parse("X");
        }

        static XmlReaderSettings ReaderSettings()
        {
            var settings = new XmlReaderSettings();
            settings.DtdProcessing = DtdProcessing.Prohibit;
            settings.XmlResolver = null;
            settings.IgnoreComments = true;
            return settings;
        }

        static List<XmlElement> Children(XmlElement parent, string name)
        {
            var list = new List<XmlElement>();
            if (parent == null) return list;
            foreach (XmlNode node in parent.ChildNodes)
            {
                XmlElement element = node as XmlElement;
                if (element != null && NameIs(element, name)) list.Add(element);
            }
            return list;
        }

        static XmlElement FirstChild(XmlElement parent, string name)
        {
            List<XmlElement> list = Children(parent, name);
            return list.Count == 0 ? null : list[0];
        }

        static bool NameIs(XmlElement element, string name)
        {
            return element != null && string.Equals(element.LocalName, name, StringComparison.OrdinalIgnoreCase);
        }

        static string Attr(XmlElement element, string name)
        {
            if (element == null || !element.HasAttributes) return null;
            foreach (XmlAttribute attribute in element.Attributes)
                if (string.Equals(attribute.LocalName, name, StringComparison.OrdinalIgnoreCase))
                    return attribute.Value;
            return null;
        }

        static string ChildText(XmlElement parent, string name)
        {
            XmlElement child = FirstChild(parent, name);
            if (child == null || child.InnerText == null) return null;
            string text = child.InnerText.Trim();
            return text.Length == 0 ? null : text;
        }

        static string TextOrEmpty(string text)
        {
            return text == null ? "" : text;
        }

        static bool TryChildInt(XmlElement parent, string name, out int value)
        {
            value = 0;
            string text = ChildText(parent, name);
            if (text == null) return false;
            value = (int)ParseInteger(text);
            return true;
        }

        static bool TryChildDouble(XmlElement parent, string name, out double value)
        {
            value = 0;
            string text = ChildText(parent, name);
            if (text == null) return false;
            return double.TryParse(text, NumberStyles.Float, CultureInfo.InvariantCulture, out value);
        }

        static int OptionalIntAttr(XmlElement element, string name)
        {
            string text = Attr(element, name);
            if (text == null || text.Trim().Length == 0) return 0;
            return (int)ParseInteger(text);
        }

        static bool FlagValue(string text)
        {
            if (text == null) return false;
            text = text.Trim();
            return text == "1" || string.Equals(text, "true", StringComparison.OrdinalIgnoreCase) || string.Equals(text, "yes", StringComparison.OrdinalIgnoreCase);
        }

        public static long ParseInteger(string text)
        {
            if (text == null) throw new FormatException("A number is missing.");
            text = text.Trim();
            if (text.Length == 0) throw new FormatException("A number is missing.");
            bool negative = false;
            if (text[0] == '+') text = text.Substring(1);
            else if (text[0] == '-')
            {
                negative = true;
                text = text.Substring(1);
            }
            if (text.Length == 0) throw new FormatException("A number is missing.");
            long value;
            if (text.StartsWith("0x", StringComparison.OrdinalIgnoreCase))
                value = Convert.ToInt64(text.Substring(2), 16);
            else if (text[0] == '$')
                value = Convert.ToInt64(text.Substring(1), 16);
            else if (HasHexLetter(text))
                value = Convert.ToInt64(text, 16);
            else
                value = Convert.ToInt64(text, CultureInfo.InvariantCulture);
            return negative ? -value : value;
        }

        static ulong ParseUnsigned(string text)
        {
            text = text.Trim();
            if (text.StartsWith("0x", StringComparison.OrdinalIgnoreCase))
                return Convert.ToUInt64(text.Substring(2), 16);
            if (text.Length > 0 && text[0] == '$')
                return Convert.ToUInt64(text.Substring(1), 16);
            if (HasHexLetter(text)) return Convert.ToUInt64(text, 16);
            return Convert.ToUInt64(text, CultureInfo.InvariantCulture);
        }

        static bool HasHexLetter(string text)
        {
            for (int i = 0; i < text.Length; i++)
            {
                char c = text[i];
                if (c >= '0' && c <= '9') continue;
                if ((c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F')) return true;
                return false;
            }
            return false;
        }

        static string CanonicalId(string text)
        {
            if (text == null) return "";
            text = text.Trim();
            if (text.Length == 0) return "";
            try { return ParseInteger(text).ToString(CultureInfo.InvariantCulture); }
            catch (FormatException) { return text.ToLowerInvariant(); }
            catch (OverflowException) { return text.ToLowerInvariant(); }
        }
    }
}
