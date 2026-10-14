using System;
using System.Collections;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Text;
using System.Web.Script.Serialization;

namespace Wideband
{
    // Seed-key masks and checksum windows come from security-config.json.
    // A row with configured false is ignored. This file has no production key table.
    // Denso: nibble-swap each seed byte, then XOR the 32-bit mask (big-endian on the wire by default).
    // GM: rotate, XOR the table mask, add the offset, modulo 2^16 or 2^32.
    // Checksums sum each byte, or run CRC-32/ISO-HDLC, over start..end inclusive.
    // Store bytes inside that window count as zero. The field then receives the result.
    sealed class PreparedImage
    {
        public readonly byte[] Bytes;
        public readonly IChecksum Checksum;

        public PreparedImage(byte[] bytes, IChecksum checksum)
        {
            if (bytes == null) throw new ArgumentNullException("bytes");
            if (checksum == null) throw new ArgumentNullException("checksum");
            Bytes = bytes;
            Checksum = checksum;
        }
    }

    static class PlatformSecurityAndChecksum
    {
        public const string ValidText = "Checksum Valid";
        public const string UpdatedText = "Checksum Updated";

        static readonly object gate = new object();
        static readonly string[] BuiltInIds = new string[] { "tacoma-can", "tacoma-kline", "silverado-can", "silverado-vpw" };
        static readonly List<string> registered = new List<string>();
        static List<ChecksumEntry> checksums = new List<ChecksumEntry>();

        public static string LoadNote = "";

        public static void Install(string path)
        {
            lock (gate)
            {
                ClearRegistered();
                checksums = new List<ChecksumEntry>();
                LoadNote = "";
                try
                {
                    if (path == null || path.Trim().Length == 0)
                        throw new ArgumentException("The security config path is empty.");
                    if (!File.Exists(path)) File.WriteAllText(path, Template());
                    Parse(File.ReadAllText(path));
                }
                catch (Exception ex)
                {
                    ClearRegistered();
                    checksums = new List<ChecksumEntry>();
                    LoadNote = ex.Message == null || ex.Message.Length == 0
                        ? "The security config could not be read."
                        : ex.Message;
                }
            }
        }

        public static byte[] Reveal(byte[] seed, EcuProfile profile, ComputeKey local, Action<string> log)
        {
            if (profile == null) throw new ArgumentNullException("profile");
            if (seed == null) throw new ArgumentNullException("seed");
            byte[] key = local != null ? local(seed, profile) : SecurityKeys.Compute(seed, profile);
            if (key == null || key.Length == 0)
                throw new InvalidOperationException("The security key function returned no key.");
            if (log != null) log(HandshakeLine(seed, key));
            return key;
        }

        public static string HandshakeLine(byte[] seed, byte[] key)
        {
            return "0x27 seed " + Hex(seed) + " key " + Hex(key);
        }

        public static byte[] Denso32(byte[] seed, uint mask, bool bigEndian)
        {
            if (seed == null || seed.Length != 4)
                throw new InvalidOperationException("The Denso seed must be 4 bytes.");
            byte[] swapped = new byte[4];
            for (int i = 0; i < 4; i++)
            {
                int value = seed[i] & 0xFF;
                swapped[i] = (byte)(((value << 4) | (value >> 4)) & 0xFF);
            }
            byte[] maskBytes = Pack(mask, 4, bigEndian);
            byte[] result = new byte[4];
            for (int i = 0; i < 4; i++) result[i] = (byte)(swapped[i] ^ maskBytes[i]);
            return result;
        }

        public static byte[] Gm(byte[] seed, int bits, int rotate, bool left, uint xor, uint add, bool bigEndian)
        {
            if (bits != 16 && bits != 32)
                throw new InvalidOperationException("The GM key width must be 16 or 32 bits.");
            int width = bits / 8;
            if (seed == null || seed.Length != width)
                throw new InvalidOperationException("The GM key width does not match the seed.");
            if (rotate < 0)
                throw new InvalidOperationException("The GM rotate count cannot be negative.");
            uint mask = bits == 32 ? 0xFFFFFFFFu : 0xFFFFu;
            uint value = Unpack(seed, bigEndian) & mask;
            value = Rotate(value, bits, rotate, left);
            value = (value ^ (xor & mask)) & mask;
            value = (value + (add & mask)) & mask;
            return Pack(value, width, bigEndian);
        }

        public static string Mark(byte[] before, byte[] after)
        {
            if (before == null || after == null || before.Length != after.Length) return UpdatedText;
            for (int i = 0; i < before.Length; i++)
                if (before[i] != after[i]) return UpdatedText;
            return ValidText;
        }

        public static IChecksum ChecksumFor(string profileId, byte[] image, long origin)
        {
            if (image == null || image.Length == 0) return null;
            ChecksumEntry[] entries;
            lock (gate) { entries = checksums.ToArray(); }
            var built = new List<IChecksum>();
            for (int i = 0; i < entries.Length; i++)
            {
                ChecksumEntry entry = entries[i];
                if (!Applies(entry, profileId)) continue;
                int start;
                int length;
                int store;
                if (!TryRange(image, origin, entry, out start, out length, out store)) continue;
                built.Add(new BlockChecksum(entry.Algorithm, start, length, store, entry.BigEndian));
            }
            if (built.Count == 0) return null;
            if (built.Count == 1) return built[0];
            return new ChecksumGroup(built.ToArray());
        }

        public static PreparedImage PrepareForFlash(string profileId, string profileName, byte[] image, long origin)
        {
            if (image == null || image.Length == 0) throw new ArgumentException("The download image is empty.");
            IChecksum checksum = ChecksumFor(profileId, image, origin);
            string name = profileName == null || profileName.Length == 0 ? profileId : profileName;
            if (checksum == null)
                throw new InvalidOperationException("No checksum range is configured for " + name + ".");
            byte[] corrected = (byte[])image.Clone();
            checksum.Stamp(corrected);
            if (!checksum.Verify(corrected))
                throw new InvalidDataException("The checksum could not be stored.");
            return new PreparedImage(corrected, checksum);
        }

        static bool Applies(ChecksumEntry entry, string profileId)
        {
            if (entry == null || !entry.Configured) return false;
            if (entry.Profile.Length == 0) return true;
            if (profileId == null || profileId.Length == 0) return false;
            return string.Equals(entry.Profile, Canonical(profileId), StringComparison.OrdinalIgnoreCase);
        }

        static void Parse(string json)
        {
            if (json == null || json.Trim().Length == 0) throw new FormatException("The security config is empty.");
            var serializer = new JavaScriptSerializer();
            var root = serializer.Deserialize<Dictionary<string, object>>(json);
            if (root == null) throw new FormatException("The security config is empty.");
            List<DensoGeneration> generations = ReadGenerations(root);
            List<GmKey> keys = ReadGmKeys(root);
            List<ChecksumEntry> ranges = ReadChecksums(root);
            ReadProfiles(root, generations, keys);
            checksums = ranges;
        }

        static void ReadProfiles(Dictionary<string, object> root, List<DensoGeneration> generations, List<GmKey> keys)
        {
            object raw;
            if (!root.TryGetValue("profiles", out raw) || raw == null) return;
            var profiles = raw as Dictionary<string, object>;
            if (profiles == null) throw new FormatException("profiles is not an object.");
            foreach (KeyValuePair<string, object> pair in profiles)
            {
                var item = pair.Value as Dictionary<string, object>;
                if (item == null) throw new FormatException("A profile entry is not an object.");
                if (!Flag(item, "configured")) continue;
                string id = Canonical(pair.Key);
                if (id.Length == 0) throw new FormatException("A profile id is required.");
                string algorithm = Optional(item, "algorithm").ToLowerInvariant();
                ComputeKey compute = null;
                if (algorithm == "denso32") compute = MakeDenso(item, generations);
                else if (algorithm == "gm") compute = MakeGm(item, keys);
                else throw new FormatException(id + " needs algorithm denso32 or gm.");
                if (compute == null)
                {
                    if (LoadNote.Length == 0)
                        LoadNote = "No security key function is configured for " + id + ".";
                    continue;
                }
                Remember(id, compute);
            }
        }

        static ComputeKey MakeDenso(Dictionary<string, object> item, List<DensoGeneration> generations)
        {
            string generationId = Optional(item, "generation");
            uint mask;
            bool bigEndian;
            if (generationId.Length > 0)
            {
                DensoGeneration generation = null;
                for (int i = 0; i < generations.Count; i++)
                {
                    if (string.Equals(generations[i].Id, generationId, StringComparison.OrdinalIgnoreCase))
                        generation = generations[i];
                }
                if (generation == null || !generation.Configured) return null;
                mask = generation.Mask;
                bigEndian = generation.BigEndian;
            }
            else
            {
                if (!item.ContainsKey("mask")) return null;
                mask = Masked(item["mask"], 32);
                bigEndian = BigEndianOf(item);
            }
            uint localMask = mask;
            bool localEndian = bigEndian;
            return delegate(byte[] seed, EcuProfile profile)
            {
                return Denso32(seed, localMask, localEndian);
            };
        }

        static ComputeKey MakeGm(Dictionary<string, object> item, List<GmKey> keys)
        {
            string keyId = Optional(item, "keyId");
            GmKey found = null;
            if (keyId.Length > 0)
            {
                for (int i = 0; i < keys.Count; i++)
                {
                    if (string.Equals(keys[i].Id, keyId, StringComparison.OrdinalIgnoreCase) && keys[i].Configured)
                        found = keys[i];
                }
            }
            if (found == null) return null;
            GmKey row = found;
            return delegate(byte[] seed, EcuProfile profile)
            {
                return Gm(seed, row.Bits, row.Rotate, row.Left, row.Xor, row.Add, row.BigEndian);
            };
        }

        static List<DensoGeneration> ReadGenerations(Dictionary<string, object> root)
        {
            var list = new List<DensoGeneration>();
            object raw;
            if (!root.TryGetValue("densoGenerations", out raw) || raw == null) return list;
            var rows = raw as ArrayList;
            if (rows == null) throw new FormatException("densoGenerations is not a list.");
            foreach (object entry in rows)
            {
                var item = entry as Dictionary<string, object>;
                if (item == null) throw new FormatException("A Denso generation is not an object.");
                if (!Flag(item, "configured")) continue;
                var row = new DensoGeneration();
                row.Id = Optional(item, "id");
                if (row.Id.Length == 0) throw new FormatException("A Denso generation needs an id.");
                if (!item.ContainsKey("mask")) throw new FormatException(row.Id + " needs a mask.");
                row.Mask = Masked(item["mask"], 32);
                row.BigEndian = BigEndianOf(item);
                row.Configured = true;
                list.Add(row);
            }
            return list;
        }

        static List<GmKey> ReadGmKeys(Dictionary<string, object> root)
        {
            var list = new List<GmKey>();
            object raw;
            if (!root.TryGetValue("gmKeys", out raw) || raw == null) return list;
            var rows = raw as ArrayList;
            if (rows == null) throw new FormatException("gmKeys is not a list.");
            foreach (object entry in rows)
            {
                var item = entry as Dictionary<string, object>;
                if (item == null) throw new FormatException("A GM key is not an object.");
                if (!Flag(item, "configured")) continue;
                var row = new GmKey();
                row.Id = Optional(item, "id");
                if (row.Id.Length == 0) throw new FormatException("A GM key needs an id.");
                if (!item.ContainsKey("bits")) throw new FormatException(row.Id + " needs a bit width.");
                row.Bits = (int)Integer(item["bits"]);
                if (row.Bits != 16 && row.Bits != 32) throw new FormatException(row.Id + " must be 16 or 32 bits.");
                row.Rotate = item.ContainsKey("rotate") ? (int)Integer(item["rotate"]) : 0;
                if (row.Rotate < 0) throw new FormatException(row.Id + " has a negative rotate count.");
                string direction = Optional(item, "direction");
                if (direction.Length == 0 || string.Equals(direction, "left", StringComparison.OrdinalIgnoreCase)) row.Left = true;
                else if (string.Equals(direction, "right", StringComparison.OrdinalIgnoreCase)) row.Left = false;
                else throw new FormatException(row.Id + " direction must be left or right.");
                if (!item.ContainsKey("xor")) throw new FormatException(row.Id + " needs an xor mask.");
                row.Xor = Masked(item["xor"], row.Bits);
                row.Add = item.ContainsKey("add") ? Masked(item["add"], row.Bits) : 0;
                row.BigEndian = BigEndianOf(item);
                row.Configured = true;
                list.Add(row);
            }
            return list;
        }

        static List<ChecksumEntry> ReadChecksums(Dictionary<string, object> root)
        {
            var list = new List<ChecksumEntry>();
            object raw;
            if (!root.TryGetValue("checksums", out raw) || raw == null) return list;
            var rows = raw as ArrayList;
            if (rows == null) throw new FormatException("checksums is not a list.");
            foreach (object entry in rows)
            {
                var item = entry as Dictionary<string, object>;
                if (item == null) throw new FormatException("A checksum is not an object.");
                if (!Flag(item, "configured")) continue;
                var row = new ChecksumEntry();
                row.Profile = Canonical(Optional(item, "profile"));
                row.Algorithm = Optional(item, "algorithm").ToLowerInvariant();
                if (row.Algorithm != "sum16" && row.Algorithm != "sum32" && row.Algorithm != "crc32")
                    throw new FormatException("A checksum algorithm must be sum16, sum32, or crc32.");
                if (!item.ContainsKey("start") || !item.ContainsKey("end") || !item.ContainsKey("store"))
                    throw new FormatException("A checksum needs start, end, and store.");
                row.Start = Integer(item["start"]);
                row.End = Integer(item["end"]);
                row.Store = Integer(item["store"]);
                row.BigEndian = BigEndianOf(item);
                row.Configured = true;
                list.Add(row);
            }
            return list;
        }

        static void Remember(string profileId, ComputeKey compute)
        {
            SecurityKeys.Register(profileId, compute);
            registered.Add(profileId);
        }

        static void ClearRegistered()
        {
            for (int i = 0; i < registered.Count; i++) SecurityKeys.Clear(registered[i]);
            registered.Clear();
            for (int i = 0; i < BuiltInIds.Length; i++) SecurityKeys.Clear(BuiltInIds[i]);
        }

        static bool TryRange(byte[] image, long origin, ChecksumEntry entry, out int start, out int length, out int store)
        {
            start = 0;
            length = 0;
            store = 0;
            int startAt = Locate(image, origin, entry.Start);
            int endAt = Locate(image, origin, entry.End);
            int storeAt = Locate(image, origin, entry.Store);
            int width = Width(entry.Algorithm);
            if (startAt < 0 || endAt < startAt || storeAt < 0 || width < 1) return false;
            int span = endAt - startAt + 1;
            if (startAt + span > image.Length || storeAt + width > image.Length) return false;
            start = startAt;
            length = span;
            store = storeAt;
            return true;
        }

        static int Locate(byte[] image, long origin, long address)
        {
            if (address >= 0 && address < image.Length) return (int)address;
            long absolute = address - origin;
            if (absolute >= 0 && absolute < image.Length) return (int)absolute;
            return -1;
        }

        static int Width(string algorithm)
        {
            if (algorithm == "sum16") return 2;
            if (algorithm == "sum32" || algorithm == "crc32") return 4;
            return 0;
        }

        static uint Rotate(uint value, int bits, int count, bool left)
        {
            int n = count % bits;
            uint mask = bits == 32 ? 0xFFFFFFFFu : (uint)((1 << bits) - 1);
            value &= mask;
            if (n == 0) return value;
            if (left) return ((value << n) | (value >> (bits - n))) & mask;
            return ((value >> n) | (value << (bits - n))) & mask;
        }

        static byte[] Pack(uint value, int width, bool bigEndian)
        {
            byte[] data = new byte[width];
            for (int i = 0; i < width; i++)
            {
                int shift = bigEndian ? (width - 1 - i) * 8 : i * 8;
                data[i] = (byte)((value >> shift) & 0xFF);
            }
            return data;
        }

        static uint Unpack(byte[] data, bool bigEndian)
        {
            uint value = 0;
            for (int i = 0; i < data.Length; i++)
            {
                int shift = bigEndian ? (data.Length - 1 - i) * 8 : i * 8;
                value |= (uint)data[i] << shift;
            }
            return value;
        }

        static string Hex(byte[] data)
        {
            if (data == null || data.Length == 0) return "";
            var text = new StringBuilder(data.Length * 2);
            for (int i = 0; i < data.Length; i++) text.Append(data[i].ToString("X2"));
            return text.ToString();
        }

        static string Canonical(string id)
        {
            if (id == null) return "";
            string trimmed = id.Trim();
            for (int i = 0; i < BuiltInIds.Length; i++)
            {
                if (string.Equals(BuiltInIds[i], trimmed, StringComparison.OrdinalIgnoreCase)) return BuiltInIds[i];
            }
            return trimmed;
        }

        static string Optional(Dictionary<string, object> item, string key)
        {
            object value;
            if (!item.TryGetValue(key, out value) || value == null) return "";
            return value.ToString().Trim();
        }

        static bool Flag(Dictionary<string, object> item, string key)
        {
            object value;
            if (!item.TryGetValue(key, out value) || value == null) return false;
            if (value is bool) return (bool)value;
            string text = value.ToString().Trim();
            if (string.Equals(text, "true", StringComparison.OrdinalIgnoreCase)) return true;
            if (string.Equals(text, "false", StringComparison.OrdinalIgnoreCase)) return false;
            throw new FormatException(key + " must be true or false.");
        }

        static bool BigEndianOf(Dictionary<string, object> item)
        {
            string text = Optional(item, "endian");
            if (text.Length == 0 || string.Equals(text, "big", StringComparison.OrdinalIgnoreCase)) return true;
            if (string.Equals(text, "little", StringComparison.OrdinalIgnoreCase)) return false;
            throw new FormatException("The endian value must be big or little.");
        }

        static long Integer(object value)
        {
            if (value == null) throw new FormatException("A number is missing.");
            string text = value.ToString().Trim();
            if (text.StartsWith("0x", StringComparison.OrdinalIgnoreCase))
            {
                if (text.Length < 3) throw new FormatException("A hex number is empty.");
                return Convert.ToInt64(text.Substring(2), 16);
            }
            return Convert.ToInt64(value, CultureInfo.InvariantCulture);
        }

        static uint Masked(object value, int bits)
        {
            long parsed = Integer(value);
            if (bits >= 32) return unchecked((uint)parsed);
            uint mask = (uint)((1 << bits) - 1);
            return unchecked((uint)parsed) & mask;
        }

        static string Template()
        {
            return "{\r\n"
                + "  \"profiles\": {\r\n"
                + "    \"tacoma-can\": { \"algorithm\": \"denso32\", \"mask\": \"0x00000000\", \"generation\": \"\", \"endian\": \"big\", \"configured\": false },\r\n"
                + "    \"tacoma-kline\": { \"algorithm\": \"denso32\", \"mask\": \"0x00000000\", \"generation\": \"\", \"endian\": \"big\", \"configured\": false },\r\n"
                + "    \"silverado-can\": { \"algorithm\": \"gm\", \"keyId\": \"\", \"configured\": false },\r\n"
                + "    \"silverado-vpw\": { \"algorithm\": \"gm\", \"keyId\": \"\", \"configured\": false }\r\n"
                + "  },\r\n"
                + "  \"densoGenerations\": [ { \"id\": \"example\", \"mask\": \"0x00000000\", \"endian\": \"big\", \"configured\": false } ],\r\n"
                + "  \"gmKeys\": [ { \"id\": \"example\", \"bits\": 16, \"rotate\": 1, \"direction\": \"left\", \"xor\": \"0x0000\", \"add\": 0, \"endian\": \"big\", \"configured\": false } ],\r\n"
                + "  \"checksums\": [ { \"id\": \"example\", \"profile\": \"\", \"algorithm\": \"crc32\", \"start\": \"0x0\", \"end\": \"0x0\", \"store\": \"0x0\", \"endian\": \"big\", \"configured\": false } ]\r\n"
                + "}\r\n";
        }

        sealed class DensoGeneration
        {
            public string Id = "";
            public uint Mask;
            public bool BigEndian = true;
            public bool Configured;
        }

        sealed class GmKey
        {
            public string Id = "";
            public int Bits;
            public int Rotate;
            public bool Left = true;
            public uint Xor;
            public uint Add;
            public bool BigEndian = true;
            public bool Configured;
        }

        sealed class ChecksumEntry
        {
            public string Profile = "";
            public string Algorithm = "";
            public long Start;
            public long End;
            public long Store;
            public bool BigEndian = true;
            public bool Configured;
        }
    }

    sealed class BlockChecksum : IChecksum
    {
        public readonly string Algorithm;
        public readonly int RangeStart;
        public readonly int RangeLength;
        public readonly int FieldOffset;
        public readonly bool BigEndian;

        public BlockChecksum(string algorithm, int rangeStart, int rangeLength, int fieldOffset, bool bigEndian)
        {
            if (rangeStart < 0 || rangeLength < 1 || fieldOffset < 0)
                throw new ArgumentOutOfRangeException("rangeStart", "The checksum range is not valid.");
            Algorithm = algorithm;
            RangeStart = rangeStart;
            RangeLength = rangeLength;
            FieldOffset = fieldOffset;
            BigEndian = bigEndian;
        }

        public string Name
        {
            get { return Algorithm; }
        }

        public bool Verify(byte[] image)
        {
            byte[] actual;
            try { actual = Compute(image); }
            catch (ArgumentException) { return false; }
            if (image == null || FieldOffset + actual.Length > image.Length) return false;
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
            int width = Algorithm == "sum16" ? 2 : 4;
            int local = FieldOffset - RangeStart;
            if (local >= 0 && local < window.Length)
            {
                int count = width;
                if (local + count > window.Length) count = window.Length - local;
                for (int i = 0; i < count; i++) window[local + i] = 0;
            }
            if (Algorithm == "crc32")
                return Pack(RegionChecksum.Run(CrcKind.Crc32IsoHdlc, window), 4);
            uint sum = 0;
            for (int i = 0; i < window.Length; i++) sum += window[i];
            if (Algorithm == "sum16") return Pack(sum & 0xFFFF, 2);
            if (Algorithm == "sum32") return Pack(sum, 4);
            throw new ArgumentOutOfRangeException("image", "The checksum algorithm is not supported.");
        }

        byte[] Pack(uint value, int width)
        {
            byte[] data = new byte[width];
            for (int i = 0; i < width; i++)
            {
                int shift = BigEndian ? (width - 1 - i) * 8 : i * 8;
                data[i] = (byte)((value >> shift) & 0xFF);
            }
            return data;
        }
    }

    sealed class ChecksumGroup : IChecksum
    {
        readonly IChecksum[] parts;

        public ChecksumGroup(IChecksum[] parts)
        {
            if (parts == null || parts.Length == 0) throw new ArgumentException("A checksum group needs a range.");
            this.parts = parts;
        }

        public string Name
        {
            get { return "group"; }
        }

        public bool Verify(byte[] image)
        {
            for (int i = 0; i < parts.Length; i++)
                if (!parts[i].Verify(image)) return false;
            return true;
        }

        public void Stamp(byte[] image)
        {
            for (int i = 0; i < parts.Length; i++) parts[i].Stamp(image);
        }
    }
}
