using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;
using Microsoft.Win32;

namespace Wideband
{
    // One J2534 v04.04 Pass-Thru library found in the registry or supplied by the user.
    sealed class J2534Device
    {
        public string Name = "";
        public string Vendor = "";
        public string LibraryPath = "";
        public string Source = "";
        public ushort Machine;
        public bool Exists;

        public const ushort MachineX86 = 0x014C;
        public const ushort MachineX64 = 0x8664;

        // J2534Host.exe is x86. A library is usable when it exists and is not a 64-bit image.
        public bool Usable
        {
            get { return Exists && Machine != MachineX64; }
        }

        public override string ToString()
        {
            string text = Name.Length > 0 ? Name : Path.GetFileName(LibraryPath);
            if (Vendor.Length > 0 && text.IndexOf(Vendor, StringComparison.OrdinalIgnoreCase) < 0)
                text += " (" + Vendor + ")";
            if (!Exists) text += " - DLL not found";
            else if (Machine == MachineX64) text += " - 64-bit DLL, not usable";
            return text;
        }
    }

    // Enumerates SAE J2534-1 v04.04 devices. Each subkey of PassThruSupport.04.04 is one device
    // with Name, Vendor, and FunctionLibrary values. Both the WOW6432Node and native roots are read.
    static class J2534RegistryScanner
    {
        public const string Wow64Root = @"SOFTWARE\WOW6432Node\PassThruSupport.04.04";
        public const string NativeRoot = @"SOFTWARE\PassThruSupport.04.04";
        public const string VfTunerLibrary = @"C:\Program Files (x86)\VF Tuner\esp32.dll";

        public static List<J2534Device> Scan()
        {
            var found = new List<J2534Device>();
            ReadRoot(found, Wow64Root);
            ReadRoot(found, NativeRoot);
            return found;
        }

        // Registry devices first, then the VF Tuner library and any extra fallback path.
        public static List<J2534Device> WithFallbacks(string extraLibrary)
        {
            var list = new List<J2534Device>();
            foreach (J2534Device device in Scan()) Add(list, device);
            Add(list, Describe("VF Tuner", "VF Tuner", VfTunerLibrary, "fallback"));
            if (extraLibrary != null && extraLibrary.Trim().Length > 0)
                Add(list, Describe("VF Tuner", "VF Tuner", extraLibrary.Trim(), "fallback"));
            return list;
        }

        public static J2534Device Describe(string name, string vendor, string libraryPath, string source)
        {
            var device = new J2534Device();
            device.Name = name == null ? "" : name.Trim();
            device.Vendor = vendor == null ? "" : vendor.Trim();
            device.LibraryPath = libraryPath == null ? "" : libraryPath.Trim();
            device.Source = source == null ? "" : source;
            device.Exists = device.LibraryPath.Length > 0 && File.Exists(device.LibraryPath);
            device.Machine = device.Exists ? PeMachine(device.LibraryPath) : (ushort)0;
            return device;
        }

        // Adds a device unless the same library path is already listed.
        public static bool Add(List<J2534Device> list, J2534Device device)
        {
            if (list == null || device == null || device.LibraryPath.Length == 0) return false;
            for (int i = 0; i < list.Count; i++)
            {
                if (string.Equals(list[i].LibraryPath, device.LibraryPath, StringComparison.OrdinalIgnoreCase))
                    return false;
            }
            list.Add(device);
            return true;
        }

        // Returns the IMAGE_FILE_HEADER Machine field, or 0 when the file is not a PE image.
        public static ushort PeMachine(string path)
        {
            try
            {
                using (var stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite))
                {
                    var head = new byte[64];
                    if (stream.Read(head, 0, head.Length) != head.Length) return 0;
                    if (head[0] != (byte)'M' || head[1] != (byte)'Z') return 0;
                    int offset = BitConverter.ToInt32(head, 0x3C);
                    if (offset < 64 || offset > 64 * 1024 || offset + 6 > stream.Length) return 0;
                    stream.Seek(offset, SeekOrigin.Begin);
                    var pe = new byte[6];
                    if (stream.Read(pe, 0, pe.Length) != pe.Length) return 0;
                    if (pe[0] != (byte)'P' || pe[1] != (byte)'E' || pe[2] != 0 || pe[3] != 0) return 0;
                    return (ushort)(pe[4] | (pe[5] << 8));
                }
            }
            catch (Exception)
            {
                return 0;
            }
        }

        static void ReadRoot(List<J2534Device> found, string rootPath)
        {
            RegistryKey hive = null;
            RegistryKey root = null;
            try
            {
                hive = RegistryKey.OpenBaseKey(RegistryHive.LocalMachine, RegistryView.Registry64);
                root = hive.OpenSubKey(rootPath, false);
                if (root == null) return;
                foreach (string child in root.GetSubKeyNames())
                {
                    using (RegistryKey key = root.OpenSubKey(child, false))
                    {
                        if (key == null) continue;
                        string library = Value(key, "FunctionLibrary");
                        if (library.Length == 0) continue;
                        string name = Value(key, "Name");
                        if (name.Length == 0) name = child;
                        J2534Device device = Describe(name, Value(key, "Vendor"), Environment.ExpandEnvironmentVariables(library), "HKLM\\" + rootPath);
                        Add(found, device);
                    }
                }
            }
            catch (Exception)
            {
            }
            finally
            {
                if (root != null) root.Dispose();
                if (hive != null) hive.Dispose();
            }
        }

        static string Value(RegistryKey key, string name)
        {
            object value = key.GetValue(name);
            return value == null ? "" : value.ToString().Trim();
        }
    }

    // CAN transport only. It does not program an ECU, correct a checksum, or send a frame on its own.
    sealed class CanFrame
    {
        public uint Id;
        public byte[] Data = new byte[0];
        public uint Timestamp;
        public bool Extended;
    }

    sealed class CanChannelSettings
    {
        public int Bitrate = 500000;
        public bool ExtendedId = false;
        public uint FilterId = 0x7E8;
        public uint FilterMask = 0x7FF;
        public string J2534LibraryPath = @"C:\Program Files (x86)\VF Tuner\esp32.dll";
        public string SocketInterface = "can0";
        public uint ProtocolId = 5;
        public uint ConnectFlags = 0;
        public byte Tester = 0xF1;
        public byte Ecu = 0x10;
    }

    sealed class CanLinkException : Exception
    {
        public readonly int Status;

        public CanLinkException(string message)
            : this(message, -1)
        {
        }

        public CanLinkException(string message, int status)
            : base(message)
        {
            Status = status;
        }
    }

    interface ICanLink : IDisposable
    {
        void Open(CanChannelSettings settings);
        void Close();
        bool TryRead(out CanFrame frame);
        bool TryWrite(CanFrame frame);
    }

    static class CanLinks
    {
        public static ICanLink Create(string backend)
        {
            if (string.Equals(backend, "j2534", StringComparison.OrdinalIgnoreCase))
                return new J2534CanLink();
            if (string.Equals(backend, "socketcan", StringComparison.OrdinalIgnoreCase))
                return new SocketCanLink();
            throw new ArgumentException("The CAN backend must be j2534 or socketcan.");
        }
    }

    // Linux can_frame is 16 bytes: little-endian id, DLC, pad, then 8 data bytes.
    static class SocketCanFrame
    {
        public const uint EffFlag = 0x80000000;
        public const uint RtrFlag = 0x40000000;
        public const uint ErrFlag = 0x20000000;

        public static byte[] Pack(uint id, byte[] data, bool extended)
        {
            if (data == null) data = new byte[0];
            if (data.Length > 8)
                throw new ArgumentException("A classic CAN frame holds at most 8 data bytes.");
            if (!extended && id > 0x7FF)
                throw new ArgumentOutOfRangeException("id", "An 11-bit CAN id must be 0x7FF or below.");
            if (extended && id > 0x1FFFFFFF)
                throw new ArgumentOutOfRangeException("id", "A 29-bit CAN id must be 0x1FFFFFFF or below.");
            var frame = new byte[16];
            uint wire = id;
            if (extended) wire |= EffFlag;
            frame[0] = (byte)(wire & 0xFF);
            frame[1] = (byte)((wire >> 8) & 0xFF);
            frame[2] = (byte)((wire >> 16) & 0xFF);
            frame[3] = (byte)((wire >> 24) & 0xFF);
            frame[4] = (byte)data.Length;
            for (int i = 0; i < data.Length; i++) frame[8 + i] = data[i];
            return frame;
        }

        public static void Unpack(byte[] frame, out uint id, out byte[] data, out bool extended)
        {
            uint wire = (uint)frame[0]
                | ((uint)frame[1] << 8)
                | ((uint)frame[2] << 16)
                | ((uint)frame[3] << 24);
            extended = (wire & EffFlag) != 0;
            id = wire & (extended ? 0x1FFFFFFFu : 0x7FFu);
            int length = frame[4];
            if (length > 8) length = 8;
            data = new byte[length];
            for (int i = 0; i < length; i++) data[i] = frame[8 + i];
        }
    }

    sealed class J2534CanLink : ICanLink, IRawBus
    {
        readonly object gate = new object();
        readonly StringBuilder errors = new StringBuilder();
        Process process;
        StreamWriter stdin;
        StreamReader stdout;
        bool channelOpen;
        bool extended;
        bool rawBus;
        uint protocolId = 5;
        public uint DeviceId;
        public uint ChannelId;
        public string Firmware = "";
        public string LibraryVersion = "";
        public string ApiVersion = "";

        public void Open(CanChannelSettings settings)
        {
            if (settings == null) throw new ArgumentNullException("settings");
            CheckCommon(settings);
            if (settings.J2534LibraryPath == null || settings.J2534LibraryPath.Trim().Length == 0)
                throw new ArgumentException("The J2534 library path is required.");
            if (settings.J2534LibraryPath.IndexOf('\r') >= 0 || settings.J2534LibraryPath.IndexOf('\n') >= 0)
                throw new ArgumentException("The J2534 library path is not valid.");
            lock (gate)
            {
                CloseLocked();
                StartHost(settings.J2534LibraryPath);
                try
                {
                    Dictionary<string, object> opened = Command("OPEN " + settings.J2534LibraryPath, 15000);
                    RequireOk(opened);
                    DeviceId = ReadIdValue(opened, "device_id");
                    uint protocol = settings.ProtocolId == 0 ? 5u : settings.ProtocolId;
                    string connectLine = "CONNECT " + settings.Bitrate.ToString(CultureInfo.InvariantCulture)
                        + " " + (settings.ExtendedId ? "1" : "0")
                        + " " + protocol.ToString(CultureInfo.InvariantCulture)
                        + " " + settings.ConnectFlags.ToString("X", CultureInfo.InvariantCulture);
                    Dictionary<string, object> connected = Command(connectLine, 15000);
                    RequireOk(connected);
                    ChannelId = ReadIdValue(connected, "channel_id");
                    uint reportedDevice = ReadIdValue(connected, "device_id");
                    if (reportedDevice != 0) DeviceId = reportedDevice;
                    if (protocol == 4)
                    {
                        byte[] wake = KwpFrame.Pack(settings.Ecu, settings.Tester, new byte[] { 0x81 });
                        RequireOk(Command("INIT " + ToHex(wake), 15000));
                    }
                    if (protocol == 5)
                        RequireOk(Command("FILTER " + settings.FilterId.ToString("X") + " " + settings.FilterMask.ToString("X"), 15000));
                    else
                        RequireOk(Command("FILTER 00 00", 15000));
                    extended = settings.ExtendedId && protocol == 5;
                    rawBus = protocol != 5;
                    protocolId = protocol;
                    channelOpen = true;
                    TryReadVersion();
                }
                catch
                {
                    CloseLocked();
                    throw;
                }
            }
        }

        public void Close()
        {
            lock (gate) CloseLocked();
        }

        public void Abort()
        {
            Process current = process;
            try
            {
                if (current != null && !current.HasExited) current.Kill();
            }
            catch (Exception)
            {
            }
        }

        public void Dispose()
        {
            Close();
        }

        public bool TryRead(out CanFrame frame)
        {
            frame = null;
            lock (gate)
            {
                if (!channelOpen) throw new InvalidOperationException("The channel is not open.");
                if (rawBus) throw new InvalidOperationException("This channel is not CAN.");
                Dictionary<string, object> response = Command("READ", 5000);
                if (Flag(response, "empty")) return false;
                RequireOk(response);
                string idText = Text(response, "id");
                if (idText.Length == 0) throw new CanLinkException("The CAN host returned a frame without an id.");
                frame = new CanFrame();
                frame.Id = uint.Parse(idText, NumberStyles.HexNumber, CultureInfo.InvariantCulture);
                frame.Extended = Flag(response, "extended");
                frame.Data = ParseHexBytes(Text(response, "data"));
                frame.Timestamp = Number(response, "timestamp");
                return true;
            }
        }

        public bool TryWrite(CanFrame frame)
        {
            if (frame == null) throw new ArgumentNullException("frame");
            if (rawBus) throw new InvalidOperationException("This channel is not CAN.");
            byte[] data = frame.Data ?? new byte[0];
            if (data.Length > 8)
                throw new ArgumentException("A classic CAN frame holds at most 8 data bytes.");
            lock (gate)
            {
                if (!channelOpen) throw new InvalidOperationException("The CAN channel is not open.");
                if (!extended && frame.Id > 0x7FF)
                    throw new ArgumentOutOfRangeException("frame", "An 11-bit CAN id must be 0x7FF or below.");
                if (extended && frame.Id > 0x1FFFFFFF)
                    throw new ArgumentOutOfRangeException("frame", "A 29-bit CAN id must be 0x1FFFFFFF or below.");
                string line = "WRITE " + frame.Id.ToString("X");
                if (data.Length > 0) line += " " + ToHex(data);
                Dictionary<string, object> response = Command(line, 5000);
                if (Flag(response, "full")) return false;
                RequireOk(response);
                return true;
            }
        }

        public bool TryReadRaw(out byte[] data)
        {
            data = null;
            lock (gate)
            {
                if (!channelOpen) throw new InvalidOperationException("The channel is not open.");
                if (!rawBus) throw new InvalidOperationException("This channel is CAN.");
                Dictionary<string, object> response = Command("READ", 5000);
                if (Flag(response, "empty")) return false;
                RequireOk(response);
                data = ParseRawHex(Text(response, "data"));
                return data.Length > 0;
            }
        }

        public bool TryWriteRaw(byte[] data)
        {
            if (data == null || data.Length == 0) throw new ArgumentException("The frame is empty.");
            int max = protocolId == 1 ? 11 : 260;
            if (data.Length > max)
                throw new ArgumentException("The frame is too long for this protocol.");
            lock (gate)
            {
                if (!channelOpen) throw new InvalidOperationException("The channel is not open.");
                if (!rawBus) throw new InvalidOperationException("This channel is CAN.");
                Dictionary<string, object> response = Command("WRITE " + ToHex(data), 5000);
                if (Flag(response, "full")) return false;
                RequireOk(response);
                return true;
            }
        }

        void TryReadVersion()
        {
            try
            {
                Dictionary<string, object> response = Command("VERSION", 15000);
                if (!Flag(response, "ok")) return;
                Firmware = Text(response, "firmware");
                LibraryVersion = Text(response, "dll");
                ApiVersion = Text(response, "api");
                uint reportedDevice = ReadIdValue(response, "device_id");
                if (reportedDevice != 0) DeviceId = reportedDevice;
            }
            catch (Exception)
            {
            }
        }

        static uint ReadIdValue(Dictionary<string, object> response, string key)
        {
            uint value;
            if (uint.TryParse(Text(response, key), NumberStyles.Integer, CultureInfo.InvariantCulture, out value))
                return value;
            return 0;
        }

        void StartHost(string libraryPath)
        {
            string hostPath = Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "J2534Host.exe");
            if (!File.Exists(hostPath))
                throw new CanLinkException("J2534Host.exe is missing beside Wideband.exe.");
            var start = new ProcessStartInfo();
            start.FileName = hostPath;
            start.UseShellExecute = false;
            start.CreateNoWindow = true;
            start.RedirectStandardInput = true;
            start.RedirectStandardOutput = true;
            start.RedirectStandardError = true;
            string folder = Path.GetDirectoryName(libraryPath);
            if (!string.IsNullOrEmpty(folder) && Directory.Exists(folder))
                start.WorkingDirectory = folder;
            process = new Process();
            process.StartInfo = start;
            process.ErrorDataReceived += OnError;
            lock (errors) errors.Length = 0;
            if (!process.Start())
                throw new CanLinkException("The CAN host did not start.");
            process.BeginErrorReadLine();
            stdin = process.StandardInput;
            stdout = process.StandardOutput;
            stdin.AutoFlush = true;
        }

        void OnError(object sender, DataReceivedEventArgs args)
        {
            if (args.Data == null) return;
            lock (errors) errors.AppendLine(args.Data);
        }

        Dictionary<string, object> Command(string line, int timeoutMs)
        {
            if (process == null || process.HasExited)
                throw new CanLinkException("The CAN host is not running. " + ErrorText());
            stdin.WriteLine(line);
            string response = ReadResponse(timeoutMs);
            int start = response.IndexOf('{');
            int end = response.LastIndexOf('}');
            if (start < 0 || end < start)
                throw new CanLinkException("The CAN host returned an unexpected result.");
            var serializer = new JavaScriptSerializer();
            var payload = serializer.Deserialize<Dictionary<string, object>>(response.Substring(start, end - start + 1));
            if (payload == null)
                throw new CanLinkException("The CAN host returned an unexpected result.");
            return payload;
        }

        string ReadResponse(int timeoutMs)
        {
            string line = null;
            Exception error = null;
            var thread = new Thread(delegate()
            {
                try { line = stdout.ReadLine(); }
                catch (Exception ex) { error = ex; }
            });
            thread.IsBackground = true;
            thread.Start();
            if (!thread.Join(timeoutMs))
                throw new CanLinkException("The CAN host did not answer.");
            if (error != null) throw new CanLinkException(error.Message);
            if (line == null) throw new CanLinkException("The CAN host closed. " + ErrorText());
            if (line.Length > 0 && line[0] == '\uFEFF') line = line.Substring(1);
            return line;
        }

        void CloseLocked()
        {
            channelOpen = false;
            extended = false;
            rawBus = false;
            protocolId = 5;
            Process current = process;
            process = null;
            if (current == null) return;
            try
            {
                if (!current.HasExited && stdin != null)
                {
                    stdin.WriteLine("CLOSE");
                    stdin.WriteLine("QUIT");
                    current.WaitForExit(3000);
                }
            }
            catch (Exception)
            {
            }
            try
            {
                if (!current.HasExited) current.Kill();
            }
            catch (Exception)
            {
            }
            try { current.Dispose(); }
            catch (Exception) { }
            stdin = null;
            stdout = null;
        }

        string ErrorText()
        {
            lock (errors) return errors.ToString().Trim();
        }

        static void RequireOk(Dictionary<string, object> response)
        {
            if (Flag(response, "ok")) return;
            string error = Text(response, "error");
            if (error.Length == 0) error = "The CAN channel did not open.";
            throw new CanLinkException(error, StatusOf(response));
        }

        static bool Flag(Dictionary<string, object> map, string key)
        {
            object value;
            if (map == null || !map.TryGetValue(key, out value) || value == null) return false;
            if (value is bool) return (bool)value;
            return string.Equals(value.ToString(), "true", StringComparison.OrdinalIgnoreCase);
        }

        static string Text(Dictionary<string, object> map, string key)
        {
            object value;
            if (map != null && map.TryGetValue(key, out value) && value != null) return value.ToString();
            return "";
        }

        static int StatusOf(Dictionary<string, object> map)
        {
            object value;
            if (map == null || !map.TryGetValue("status", out value) || value == null) return -1;
            try { return Convert.ToInt32(value, CultureInfo.InvariantCulture); }
            catch (Exception) { return -1; }
        }

        static uint Number(Dictionary<string, object> map, string key)
        {
            object value;
            if (map == null || !map.TryGetValue(key, out value) || value == null) return 0;
            try { return Convert.ToUInt32(value, CultureInfo.InvariantCulture); }
            catch (Exception) { return 0; }
        }

        static byte[] ParseHexBytes(string hex)
        {
            if (hex == null || hex.Length == 0) return new byte[0];
            if ((hex.Length % 2) != 0 || hex.Length > 16)
                throw new CanLinkException("The CAN host returned a frame with unexpected data.");
            var data = new byte[hex.Length / 2];
            for (int i = 0; i < data.Length; i++)
                data[i] = byte.Parse(hex.Substring(i * 2, 2), NumberStyles.HexNumber, CultureInfo.InvariantCulture);
            return data;
        }

        static string ToHex(byte[] data)
        {
            var hex = new StringBuilder(data.Length * 2);
            for (int i = 0; i < data.Length; i++) hex.Append(data[i].ToString("X2"));
            return hex.ToString();
        }

        static byte[] ParseRawHex(string hex)
        {
            if (hex == null || hex.Length == 0) return new byte[0];
            if ((hex.Length % 2) != 0 || hex.Length > 520)
                throw new CanLinkException("The cable host returned a frame with unexpected data.");
            var data = new byte[hex.Length / 2];
            for (int i = 0; i < data.Length; i++)
                data[i] = byte.Parse(hex.Substring(i * 2, 2), NumberStyles.HexNumber, CultureInfo.InvariantCulture);
            return data;
        }

        static void CheckCommon(CanChannelSettings settings)
        {
            if (settings.Bitrate < 1 || settings.Bitrate > 1000000)
                throw new ArgumentOutOfRangeException("settings", "Bitrate must be between 1 and 1000000.");
            uint protocol = settings.ProtocolId == 0 ? 5u : settings.ProtocolId;
            if (protocol != 1 && protocol != 4 && protocol != 5)
                throw new ArgumentOutOfRangeException("settings", "Protocol must be J1850 VPW, ISO 14230, or CAN.");
            if (protocol != 5) return;
            uint cap = settings.ExtendedId ? 0x1FFFFFFFu : 0x7FFu;
            if (settings.FilterId > cap || settings.FilterMask > cap)
                throw new ArgumentOutOfRangeException("settings", "The acceptance filter does not fit the selected identifier size.");
        }
    }

    sealed class SocketCanLink : ICanLink
    {
        int fd = -1;
        bool extended;

        public void Open(CanChannelSettings settings)
        {
            if (settings == null) throw new ArgumentNullException("settings");
            if (!IsUnix())
                throw new PlatformNotSupportedException("SocketCAN is only available on Linux.");
            if (settings.Bitrate < 1 || settings.Bitrate > 1000000)
                throw new ArgumentOutOfRangeException("settings", "Bitrate must be between 1 and 1000000.");
            uint cap = settings.ExtendedId ? 0x1FFFFFFFu : 0x7FFu;
            if (settings.FilterId > cap || settings.FilterMask > cap)
                throw new ArgumentOutOfRangeException("settings", "The acceptance filter does not fit the selected identifier size.");
            if (!ValidInterface(settings.SocketInterface))
                throw new ArgumentException("The SocketCAN interface name is not valid.");
            OpenUnix(settings);
        }

        public void Close()
        {
            int current = fd;
            fd = -1;
            extended = false;
            if (current < 0) return;
            if (!IsUnix()) return;
            UnixCan.CloseFd(current);
        }

        public void Dispose()
        {
            Close();
        }

        public bool TryRead(out CanFrame frame)
        {
            frame = null;
            if (fd < 0) throw new InvalidOperationException("The CAN channel is not open.");
            byte[] buffer = new byte[16];
            int errno;
            int read = ReadOnce(buffer, out errno);
            if (read < 0)
            {
                if (errno == 11) return false;
                throw new CanLinkException("CAN read failed (errno " + errno + ").");
            }
            if (read != 16) return false;
            frame = new CanFrame();
            byte[] data;
            bool frameExtended;
            uint id;
            SocketCanFrame.Unpack(buffer, out id, out data, out frameExtended);
            frame.Id = id;
            frame.Data = data;
            frame.Extended = frameExtended;
            return true;
        }

        public bool TryWrite(CanFrame frame)
        {
            if (frame == null) throw new ArgumentNullException("frame");
            if (fd < 0) throw new InvalidOperationException("The CAN channel is not open.");
            byte[] packed = SocketCanFrame.Pack(frame.Id, frame.Data ?? new byte[0], extended);
            int errno;
            int written = WriteOnce(packed, out errno);
            if (written < 0)
            {
                if (errno == 11 || errno == 105) return false;
                throw new CanLinkException("CAN write failed (errno " + errno + ").");
            }
            return written == 16;
        }

        void OpenUnix(CanChannelSettings settings)
        {
            Close();
            // Bitrate is a link property, not a socket option.
            string iface = settings.SocketInterface;
            RunIp("link set " + iface + " down");
            RunIp("link set " + iface + " type can bitrate " + settings.Bitrate.ToString(CultureInfo.InvariantCulture));
            RunIp("link set " + iface + " up");
            int socket = UnixCan.OpenRaw();
            if (socket < 0)
                throw new CanLinkException("SocketCAN did not open (errno " + Marshal.GetLastWin32Error() + ").");
            try
            {
                int index = UnixCan.InterfaceIndex(socket, iface);
                UnixCan.Bind(socket, index);
                uint mask = settings.FilterMask | SocketCanFrame.EffFlag | SocketCanFrame.RtrFlag | SocketCanFrame.ErrFlag;
                uint id = settings.FilterId;
                if (settings.ExtendedId) id |= SocketCanFrame.EffFlag;
                UnixCan.SetFilter(socket, id, mask);
                UnixCan.SetNonBlocking(socket);
                extended = settings.ExtendedId;
                fd = socket;
                socket = -1;
            }
            finally
            {
                if (socket >= 0) UnixCan.CloseFd(socket);
            }
        }

        int ReadOnce(byte[] buffer, out int errno)
        {
            errno = 0;
            for (int attempt = 0; attempt < 2; attempt++)
            {
                int read = UnixCan.ReadFd(fd, buffer, buffer.Length);
                if (read >= 0) return read;
                errno = Marshal.GetLastWin32Error();
                if (errno != 4) return -1;
            }
            return -1;
        }

        int WriteOnce(byte[] buffer, out int errno)
        {
            errno = 0;
            for (int attempt = 0; attempt < 2; attempt++)
            {
                int written = UnixCan.WriteFd(fd, buffer, buffer.Length);
                if (written >= 0) return written;
                errno = Marshal.GetLastWin32Error();
                if (errno != 4) return -1;
            }
            return -1;
        }

        static void RunIp(string arguments)
        {
            var start = new ProcessStartInfo();
            start.FileName = "ip";
            start.Arguments = arguments;
            start.UseShellExecute = false;
            start.CreateNoWindow = true;
            start.RedirectStandardOutput = true;
            start.RedirectStandardError = true;
            using (Process process = Process.Start(start))
            {
                string error = process.StandardError.ReadToEnd();
                process.StandardOutput.ReadToEnd();
                if (!process.WaitForExit(8000))
                {
                    try { process.Kill(); }
                    catch (Exception) { }
                    throw new CanLinkException("Setting the SocketCAN bitrate timed out.");
                }
                if (process.ExitCode != 0)
                {
                    string detail = error == null ? "" : error.Trim();
                    throw new CanLinkException(detail.Length > 0 ? detail : "SocketCAN bitrate was not set.");
                }
            }
        }

        static bool ValidInterface(string name)
        {
            if (name == null || name.Length == 0 || name.Length > 15) return false;
            for (int i = 0; i < name.Length; i++)
            {
                char c = name[i];
                bool ok = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9')
                    || c == '_' || c == '-' || c == '.';
                if (!ok) return false;
            }
            return true;
        }

        static bool IsUnix()
        {
            int platform = (int)Environment.OSVersion.Platform;
            return platform == 4 || platform == 6 || platform == 128;
        }

        // Reached only after Open confirms the process is on Linux.
        static class UnixCan
        {
            const int PfCan = 29;
            const int SockRaw = 3;
            const int CanRaw = 1;
            const int SolCanRaw = 101;
            const int CanRawFilter = 1;
            const int FGetFl = 3;
            const int FSetFl = 4;
            const int ONonBlock = 2048;

            [DllImport("libc", EntryPoint = "socket", SetLastError = true)]
            static extern int socket(int domain, int type, int protocol);

            [DllImport("libc", EntryPoint = "close", SetLastError = true)]
            static extern int close(int fd);

            [DllImport("libc", EntryPoint = "bind", SetLastError = true)]
            static extern int bind(int fd, byte[] addr, int addrlen);

            [DllImport("libc", EntryPoint = "ioctl", SetLastError = true)]
            static extern int ioctl(int fd, IntPtr request, IntPtr data);

            [DllImport("libc", EntryPoint = "setsockopt", SetLastError = true)]
            static extern int setsockopt(int fd, int level, int optname, byte[] value, int length);

            [DllImport("libc", EntryPoint = "fcntl", SetLastError = true)]
            static extern int fcntl(int fd, int command, int arg);

            [DllImport("libc", EntryPoint = "read", SetLastError = true)]
            static extern int read(int fd, byte[] buffer, int count);

            [DllImport("libc", EntryPoint = "write", SetLastError = true)]
            static extern int write(int fd, byte[] buffer, int count);

            public static int OpenRaw()
            {
                return socket(PfCan, SockRaw, CanRaw);
            }

            public static void CloseFd(int fd)
            {
                close(fd);
            }

            public static int ReadFd(int fd, byte[] buffer, int count)
            {
                return read(fd, buffer, count);
            }

            public static int WriteFd(int fd, byte[] buffer, int count)
            {
                return write(fd, buffer, count);
            }

            public static int InterfaceIndex(int fd, string name)
            {
                IntPtr buffer = Marshal.AllocHGlobal(40);
                try
                {
                    for (int i = 0; i < 40; i++) Marshal.WriteByte(buffer, i, 0);
                    byte[] bytes = Encoding.ASCII.GetBytes(name);
                    Marshal.Copy(bytes, 0, buffer, bytes.Length);
                    int rc = ioctl(fd, new IntPtr(0x8933), buffer);
                    if (rc < 0)
                        throw new CanLinkException("SocketCAN interface was not found (errno " + Marshal.GetLastWin32Error() + ").");
                    return Marshal.ReadInt32(buffer, 16);
                }
                finally
                {
                    Marshal.FreeHGlobal(buffer);
                }
            }

            public static void Bind(int fd, int ifindex)
            {
                var addr = new byte[16];
                addr[0] = PfCan;
                addr[4] = (byte)(ifindex & 0xFF);
                addr[5] = (byte)((ifindex >> 8) & 0xFF);
                addr[6] = (byte)((ifindex >> 16) & 0xFF);
                addr[7] = (byte)((ifindex >> 24) & 0xFF);
                if (bind(fd, addr, addr.Length) < 0)
                    throw new CanLinkException("SocketCAN bind failed (errno " + Marshal.GetLastWin32Error() + ").");
            }

            public static void SetFilter(int fd, uint id, uint mask)
            {
                var raw = new byte[8];
                WriteLe(raw, 0, id);
                WriteLe(raw, 4, mask);
                if (setsockopt(fd, SolCanRaw, CanRawFilter, raw, raw.Length) < 0)
                    throw new CanLinkException("SocketCAN filter was not set (errno " + Marshal.GetLastWin32Error() + ").");
            }

            public static void SetNonBlocking(int fd)
            {
                int flags = fcntl(fd, FGetFl, 0);
                if (flags < 0 || fcntl(fd, FSetFl, flags | ONonBlock) < 0)
                    throw new CanLinkException("SocketCAN non-blocking mode was not set (errno " + Marshal.GetLastWin32Error() + ").");
            }

            static void WriteLe(byte[] buffer, int offset, uint value)
            {
                buffer[offset] = (byte)(value & 0xFF);
                buffer[offset + 1] = (byte)((value >> 8) & 0xFF);
                buffer[offset + 2] = (byte)((value >> 16) & 0xFF);
                buffer[offset + 3] = (byte)((value >> 24) & 0xFF);
            }
        }
    }
}
