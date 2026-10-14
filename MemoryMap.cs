using System;
using System.Collections;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Text;
using System.Web.Script.Serialization;

namespace Wideband
{
    // Raw image scaling and checksums. Encrypted vF Tuner files are refused and not decoded.
    enum ScalarType
    {
        UInt8,
        Int8,
        UInt16,
        Int16,
        UInt32,
        Int32,
        Float32
    }

    enum ScalarEndian
    {
        Little,
        Big
    }

    enum CrcKind
    {
        Crc16CcittFalse,
        Crc16Modbus,
        Crc32IsoHdlc
    }

    interface IChecksum
    {
        string Name { get; }
        bool Verify(byte[] image);
        void Stamp(byte[] image);
    }

    sealed class ScalarLayout
    {
        public int ByteOffset;
        public ScalarType Type = ScalarType.UInt8;
        public ScalarEndian Endian = ScalarEndian.Big;
        public int BitOffset;
        public int BitWidth;
        public double Factor = 1;
        public double Bias;
    }

    static class ScalarCodec
    {
        public static int Width(ScalarType type)
        {
            switch (type)
            {
                case ScalarType.UInt8:
                case ScalarType.Int8:
                    return 1;
                case ScalarType.UInt16:
                case ScalarType.Int16:
                    return 2;
                case ScalarType.UInt32:
                case ScalarType.Int32:
                case ScalarType.Float32:
                    return 4;
                default:
                    throw new ArgumentOutOfRangeException("type", "The scalar type is not supported.");
            }
        }

        public static double Read(byte[] image, ScalarLayout layout)
        {
            if (layout.Factor == 0) throw new ArgumentOutOfRangeException("layout", "The scale factor cannot be zero.");
            if (layout.Type == ScalarType.Float32)
            {
                if (layout.BitWidth != 0)
                    throw new ArgumentException("A floating-point value does not use a bit field.");
                uint bits = (uint)ReadUnsigned(image, layout.ByteOffset, 4, layout.Endian);
                byte[] local = new byte[4];
                local[0] = (byte)bits;
                local[1] = (byte)(bits >> 8);
                local[2] = (byte)(bits >> 16);
                local[3] = (byte)(bits >> 24);
                return BitConverter.ToSingle(local, 0) * layout.Factor + layout.Bias;
            }
            int width = Width(layout.Type);
            long raw = ReadUnsigned(image, layout.ByteOffset, width, layout.Endian);
            int storageBits = width * 8;
            if (layout.BitWidth > 0)
            {
                if (layout.BitOffset < 0 || layout.BitWidth > 31 || layout.BitOffset + layout.BitWidth > storageBits)
                    throw new ArgumentOutOfRangeException("layout", "The bit field does not fit in the scalar.");
                raw = (raw >> layout.BitOffset) & ((1L << layout.BitWidth) - 1);
                if (Signed(layout.Type)) raw = SignExtend(raw, layout.BitWidth);
            }
            else if (Signed(layout.Type))
            {
                raw = SignExtend(raw, storageBits);
            }
            return raw * layout.Factor + layout.Bias;
        }

        public static void Write(byte[] image, ScalarLayout layout, double engineering)
        {
            if (layout.Factor == 0) throw new ArgumentOutOfRangeException("layout", "The scale factor cannot be zero.");
            if (layout.Type == ScalarType.Float32)
            {
                if (layout.BitWidth != 0)
                    throw new ArgumentException("A floating-point value does not use a bit field.");
                float value = (float)((engineering - layout.Bias) / layout.Factor);
                byte[] local = BitConverter.GetBytes(value);
                uint bits = (uint)(local[0] | (local[1] << 8) | (local[2] << 16) | (local[3] << 24));
                WriteUnsigned(image, layout.ByteOffset, 4, layout.Endian, bits);
                return;
            }
            int width = Width(layout.Type);
            int fieldBits = layout.BitWidth > 0 ? layout.BitWidth : width * 8;
            if (layout.BitWidth > 0 && (layout.BitOffset < 0 || layout.BitWidth > 31 || layout.BitOffset + layout.BitWidth > width * 8))
                throw new ArgumentOutOfRangeException("layout", "The bit field does not fit in the scalar.");
            double scaled = (engineering - layout.Bias) / layout.Factor;
            long raw = (long)Math.Round(scaled);
            long min = Signed(layout.Type) ? -(1L << (fieldBits - 1)) : 0;
            long max = Signed(layout.Type) ? (1L << (fieldBits - 1)) - 1 : (1L << fieldBits) - 1;
            if (fieldBits >= 63) max = long.MaxValue;
            if (raw < min || raw > max)
                throw new ArgumentOutOfRangeException("engineering", "The value does not fit in the scalar.");
            long stored = ReadUnsigned(image, layout.ByteOffset, width, layout.Endian);
            if (layout.BitWidth > 0)
            {
                long mask = ((1L << layout.BitWidth) - 1) << layout.BitOffset;
                long field = raw & ((1L << layout.BitWidth) - 1);
                stored = (stored & ~mask) | (field << layout.BitOffset);
            }
            else if (Signed(layout.Type))
            {
                stored = raw & ((1L << (width * 8)) - 1);
            }
            else
            {
                stored = raw;
            }
            WriteUnsigned(image, layout.ByteOffset, width, layout.Endian, stored);
        }

        static bool Signed(ScalarType type)
        {
            return type == ScalarType.Int8 || type == ScalarType.Int16 || type == ScalarType.Int32;
        }

        static long SignExtend(long value, int bits)
        {
            long sign = 1L << (bits - 1);
            if ((value & sign) == 0) return value;
            return value - (1L << bits);
        }

        static long ReadUnsigned(byte[] image, int offset, int width, ScalarEndian endian)
        {
            if (image == null) throw new ArgumentNullException("image");
            if (offset < 0 || width < 1 || offset + width > image.Length)
                throw new ArgumentOutOfRangeException("offset", "The scalar does not fit in the image.");
            long value = 0;
            if (endian == ScalarEndian.Big)
            {
                for (int i = 0; i < width; i++) value = (value << 8) | image[offset + i];
            }
            else
            {
                for (int i = width - 1; i >= 0; i--) value = (value << 8) | image[offset + i];
            }
            return value;
        }

        static void WriteUnsigned(byte[] image, int offset, int width, ScalarEndian endian, long value)
        {
            if (image == null) throw new ArgumentNullException("image");
            if (offset < 0 || width < 1 || offset + width > image.Length)
                throw new ArgumentOutOfRangeException("offset", "The scalar does not fit in the image.");
            for (int i = 0; i < width; i++)
            {
                int shift = endian == ScalarEndian.Big ? (width - 1 - i) * 8 : i * 8;
                image[offset + i] = (byte)((value >> shift) & 0xFF);
            }
        }
    }

    sealed class TableBinding
    {
        public string Name = "";
        public int Address;
        public int Rows = 1;
        public int Columns = 1;
        public ScalarType Type = ScalarType.UInt8;
        public ScalarEndian Endian = ScalarEndian.Big;
        public double Factor = 1;
        public double Bias;
        public int BitOffset;
        public int BitWidth;

        public double[][] Read(byte[] image)
        {
            RequireShape();
            var grid = new double[Rows][];
            for (int row = 0; row < Rows; row++)
            {
                grid[row] = new double[Columns];
                for (int column = 0; column < Columns; column++)
                    grid[row][column] = ScalarCodec.Read(image, Cell(row, column));
            }
            return grid;
        }

        public void Write(byte[] image, double[][] values)
        {
            RequireShape();
            if (image == null) throw new ArgumentNullException("image");
            if (values == null || values.Length != Rows)
                throw new ArgumentException("The table row count does not match the definition.");
            for (int row = 0; row < Rows; row++)
                if (values[row] == null || values[row].Length != Columns)
                    throw new ArgumentException("The table column count does not match the definition.");
            byte[] copy = (byte[])image.Clone();
            for (int row = 0; row < Rows; row++)
                for (int column = 0; column < Columns; column++)
                    ScalarCodec.Write(copy, Cell(row, column), values[row][column]);
            Buffer.BlockCopy(copy, 0, image, 0, image.Length);
        }

        ScalarLayout Cell(int row, int column)
        {
            var layout = new ScalarLayout();
            layout.ByteOffset = Address + (row * Columns + column) * ScalarCodec.Width(Type);
            layout.Type = Type;
            layout.Endian = Endian;
            layout.Factor = Factor;
            layout.Bias = Bias;
            layout.BitOffset = BitOffset;
            layout.BitWidth = BitWidth;
            return layout;
        }

        void RequireShape()
        {
            if (Rows < 1 || Columns < 1)
                throw new ArgumentOutOfRangeException("Rows", "A table needs at least one cell.");
            if (Factor == 0)
                throw new ArgumentOutOfRangeException("Factor", "The scale factor cannot be zero.");
        }
    }

    static class MemoryDefinition
    {
        public static TableBinding Parse(string json)
        {
            if (json == null || json.Trim().Length == 0) throw new ArgumentException("The definition is empty.");
            var serializer = new JavaScriptSerializer();
            var item = serializer.Deserialize<Dictionary<string, object>>(json);
            if (item == null) throw new ArgumentException("The definition is empty.");
            return ParseItem(item);
        }

        public static List<TableBinding> ParseDocument(string json)
        {
            if (json == null || json.Trim().Length == 0) throw new ArgumentException("The definition is empty.");
            var serializer = new JavaScriptSerializer();
            var root = serializer.Deserialize<Dictionary<string, object>>(json);
            object raw;
            if (root == null || !root.TryGetValue("parameters", out raw) || !(raw is ArrayList))
                throw new ArgumentException("The definition needs a parameters list.");
            var list = new List<TableBinding>();
            foreach (object entry in (ArrayList)raw)
            {
                var item = entry as Dictionary<string, object>;
                if (item == null) throw new ArgumentException("A parameter definition is not an object.");
                list.Add(ParseItem(item));
            }
            return list;
        }

        static TableBinding ParseItem(Dictionary<string, object> item)
        {
            var binding = new TableBinding();
            binding.Name = Text(item, "name");
            if (!item.ContainsKey("address")) throw new ArgumentException("A parameter needs an address.");
            binding.Address = Integer(item["address"]);
            binding.Type = TypeOf(Text(item, "type"));
            binding.Endian = EndianOf(Text(item, "endian"));
            binding.Rows = item.ContainsKey("rows") ? Integer(item["rows"]) : 1;
            binding.Columns = item.ContainsKey("columns") ? Integer(item["columns"]) : 1;
            binding.Factor = item.ContainsKey("factor") ? Convert.ToDouble(item["factor"], CultureInfo.InvariantCulture) : 1;
            binding.Bias = item.ContainsKey("offset") ? Convert.ToDouble(item["offset"], CultureInfo.InvariantCulture) : 0;
            binding.BitOffset = item.ContainsKey("bitOffset") ? Integer(item["bitOffset"]) : 0;
            binding.BitWidth = item.ContainsKey("bitWidth") ? Integer(item["bitWidth"]) : 0;
            if (binding.Address < 0 || binding.Rows < 1 || binding.Columns < 1)
                throw new ArgumentException("The parameter shape is not valid.");
            return binding;
        }

        static string Text(Dictionary<string, object> item, string key)
        {
            object value;
            if (!item.TryGetValue(key, out value) || value == null || value.ToString().Trim().Length == 0)
                throw new ArgumentException("A parameter needs " + key + ".");
            return value.ToString().Trim();
        }

        static int Integer(object value)
        {
            return Convert.ToInt32(value, CultureInfo.InvariantCulture);
        }

        static ScalarType TypeOf(string text)
        {
            if (text == "uint8") return ScalarType.UInt8;
            if (text == "int8") return ScalarType.Int8;
            if (text == "uint16") return ScalarType.UInt16;
            if (text == "int16") return ScalarType.Int16;
            if (text == "uint32") return ScalarType.UInt32;
            if (text == "int32") return ScalarType.Int32;
            if (text == "float32") return ScalarType.Float32;
            throw new ArgumentException("The scalar type is not supported.");
        }

        static ScalarEndian EndianOf(string text)
        {
            if (text == "big") return ScalarEndian.Big;
            if (text == "little") return ScalarEndian.Little;
            throw new ArgumentException("The endian value must be big or little.");
        }
    }

    sealed class MemoryImage
    {
        public byte[] Bytes;

        public MemoryImage(byte[] bytes)
        {
            if (bytes == null) throw new ArgumentNullException("bytes");
            Bytes = bytes;
        }

        public static MemoryImage Load(string path)
        {
            if (path == null || path.Trim().Length == 0) throw new ArgumentException("The image path is empty.");
            if (path.EndsWith(".enc", StringComparison.OrdinalIgnoreCase))
                throw new InvalidDataException("Encrypted definition files are not opened.");
            byte[] data = File.ReadAllBytes(path);
            if (data.Length >= 8 && Encoding.ASCII.GetString(data, 0, 8) == "VFTBLPE1")
                throw new InvalidDataException("This profile body is opaque and is not opened.");
            return new MemoryImage(data);
        }

        public void Save(string path, IChecksum checksum)
        {
            if (path == null || path.Trim().Length == 0) throw new ArgumentException("The image path is empty.");
            if (path.EndsWith(".enc", StringComparison.OrdinalIgnoreCase))
                throw new InvalidDataException("Encrypted definition files are not written.");
            byte[] copy = (byte[])Bytes.Clone();
            if (checksum != null)
            {
                checksum.Stamp(copy);
                if (!checksum.Verify(copy))
                    throw new InvalidDataException("The checksum could not be stored.");
            }
            File.WriteAllBytes(path, copy);
            Bytes = copy;
        }
    }

    sealed class RegionChecksum : IChecksum
    {
        public readonly CrcKind Kind;
        public readonly int RangeStart;
        public readonly int RangeLength;
        public readonly int FieldOffset;
        public readonly bool BigEndian;

        public RegionChecksum(CrcKind kind, int rangeStart, int rangeLength, int fieldOffset, bool bigEndian)
        {
            if (rangeStart < 0 || rangeLength < 1 || fieldOffset < 0)
                throw new ArgumentOutOfRangeException("rangeStart", "The checksum range is not valid.");
            Kind = kind;
            RangeStart = rangeStart;
            RangeLength = rangeLength;
            FieldOffset = fieldOffset;
            BigEndian = bigEndian;
        }

        public string Name
        {
            get { return Kind.ToString(); }
        }

        public bool Verify(byte[] image)
        {
            byte[] actual;
            try { actual = Compute(image); }
            catch (ArgumentException) { return false; }
            if (FieldOffset + actual.Length > image.Length) return false;
            for (int i = 0; i < actual.Length; i++)
                if (image[FieldOffset + i] != actual[i]) return false;
            return true;
        }

        public void Stamp(byte[] image)
        {
            byte[] actual = Compute(image);
            if (FieldOffset + actual.Length > image.Length)
                throw new ArgumentOutOfRangeException("image", "The checksum field does not fit in the image.");
            Buffer.BlockCopy(actual, 0, image, FieldOffset, actual.Length);
        }

        byte[] Compute(byte[] image)
        {
            if (image == null) throw new ArgumentNullException("image");
            if (RangeStart + RangeLength > image.Length)
                throw new ArgumentOutOfRangeException("image", "The checksum range does not fit in the image.");
            byte[] window = new byte[RangeLength];
            Buffer.BlockCopy(image, RangeStart, window, 0, RangeLength);
            int width = Width(Kind);
            int local = FieldOffset - RangeStart;
            if (local >= 0 && local < window.Length)
            {
                int count = width;
                if (local + count > window.Length) count = window.Length - local;
                for (int i = 0; i < count; i++) window[local + i] = 0;
            }
            uint crc = Run(Kind, window);
            return Pack(crc, width);
        }

        byte[] Pack(uint crc, int width)
        {
            byte[] data = new byte[width];
            for (int i = 0; i < width; i++)
            {
                int shift = BigEndian ? (width - 1 - i) * 8 : i * 8;
                data[i] = (byte)((crc >> shift) & 0xFF);
            }
            return data;
        }

        public static int Width(CrcKind kind)
        {
            return kind == CrcKind.Crc32IsoHdlc ? 4 : 2;
        }

        public static uint Run(CrcKind kind, byte[] data)
        {
            if (kind == CrcKind.Crc16CcittFalse) return Crc16(data, 0xFFFF, 0x1021, false);
            if (kind == CrcKind.Crc16Modbus) return Crc16(data, 0xFFFF, 0xA001, true);
            if (kind == CrcKind.Crc32IsoHdlc) return Crc32(data);
            throw new ArgumentOutOfRangeException("kind", "The checksum kind is not supported.");
        }

        static uint Crc16(byte[] data, ushort seed, ushort polynomial, bool reflected)
        {
            ushort crc = seed;
            for (int i = 0; i < data.Length; i++)
            {
                if (!reflected)
                {
                    crc ^= (ushort)(data[i] << 8);
                    for (int bit = 0; bit < 8; bit++)
                    {
                        if ((crc & 0x8000) != 0) crc = (ushort)((crc << 1) ^ polynomial);
                        else crc <<= 1;
                    }
                }
                else
                {
                    crc ^= data[i];
                    for (int bit = 0; bit < 8; bit++)
                    {
                        if ((crc & 0x0001) != 0) crc = (ushort)((crc >> 1) ^ polynomial);
                        else crc >>= 1;
                    }
                }
            }
            return crc;
        }

        static uint Crc32(byte[] data)
        {
            uint crc = 0xFFFFFFFF;
            for (int i = 0; i < data.Length; i++)
            {
                crc ^= data[i];
                for (int bit = 0; bit < 8; bit++)
                {
                    if ((crc & 1) != 0) crc = (crc >> 1) ^ 0xEDB88320;
                    else crc >>= 1;
                }
            }
            return ~crc;
        }
    }
}
