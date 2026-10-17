using System;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

// 32-bit helper. Wideband.exe is 64-bit and cannot load esp32.dll.
// CONNECT baud extended protocol flags.
// Protocol 5 is CAN. Protocol 4 is ISO 14230. Protocol 1 is J1850 VPW.
// CAN data starts with the id, big-endian, as SAE J2534-1 requires.
// K-Line and VPW data is the frame. ISO 14230 can send FAST_INIT.
// Commands: OPEN, CONNECT, INIT, FILTER, READ [ms], WRITE, IOCTL, VERSION, CLOSE, QUIT.
// Programming voltage and periodic messages are not available.
sealed class J2534Host
{
    const int MessageBytes = 4152;
    const int ProtocolCan = 5;
    // SAE J2534-1 ISO15765: the cable segments, reassembles, and answers flow control itself.
    const int ProtocolIso15765 = 6;
    const int ProtocolKLine = 4;
    const int ProtocolVpw = 1;
    const uint Can29Bit = 0x100;
    const uint PassFilter = 1;
    const uint FlowControlFilter = 3;
    const uint Iso15765FramePad = 0x40;
    // RxStatus TX_MSG_TYPE, START_OF_MESSAGE, and TX_INDICATION mark echoes and progress
    // indications on an ISO15765 channel, not a message from the ECU.
    const uint RxIndicationMask = 0x01 | 0x02 | 0x08;
    // PASSTHRU_MSG: 24-byte header, then the 4-byte CAN id and the payload.
    const int MaxIsoTpPayload = MessageBytes - 28;
    // SAE J2534-1: FAST_INIT is 0x05. 0x03 is READ_VBATT.
    const uint ReadVbatt = 0x03;
    const uint FastInit = 0x05;
    // Drops replies still queued from an earlier request; Techstream sends it before each request.
    const uint ClearRxBuffer = 0x08;
    static readonly byte[] Zeros = new byte[MessageBytes];

    [DllImport("kernel32", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr LoadLibrary(string path);

    [DllImport("kernel32", CharSet = CharSet.Ansi, SetLastError = true)]
    static extern IntPtr GetProcAddress(IntPtr module, string name);

    [DllImport("kernel32", SetLastError = true)]
    static extern bool FreeLibrary(IntPtr module);

    [DllImport("kernel32", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool SetDllDirectory(string path);

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    delegate int OpenFn(IntPtr name, out uint deviceId);

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    delegate int CloseFn(uint deviceId);

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    delegate int ConnectFn(uint deviceId, uint protocolId, uint flags, uint baud, out uint channelId);

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    delegate int DisconnectFn(uint channelId);

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    delegate int ReadFn(uint channelId, IntPtr message, ref uint count, uint timeout);

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    delegate int WriteFn(uint channelId, IntPtr message, ref uint count, uint timeout);

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    delegate int StartFilterFn(uint channelId, uint filterType, IntPtr mask, IntPtr pattern, IntPtr flow, out uint filterId);

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    delegate int StopFilterFn(uint channelId, uint filterId);

    [UnmanagedFunctionPointer(CallingConvention.StdCall, CharSet = CharSet.Ansi)]
    delegate int ErrorFn(StringBuilder message);

    [UnmanagedFunctionPointer(CallingConvention.StdCall, CharSet = CharSet.Ansi)]
    delegate int VersionFn(uint deviceId, StringBuilder firmware, StringBuilder dll, StringBuilder api);

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    delegate int IoctlFn(uint channelId, uint ioctlId, IntPtr input, IntPtr output);

    IntPtr module;
    bool deviceOpen;
    bool channelOpen;
    bool filterOpen;
    bool extended;
    uint protocol = 5;
    uint wireFlags;
    uint deviceId;
    uint channelId;
    uint filterId;
    OpenFn openFn;
    CloseFn closeFn;
    ConnectFn connectFn;
    DisconnectFn disconnectFn;
    ReadFn readFn;
    WriteFn writeFn;
    StartFilterFn startFilterFn;
    StopFilterFn stopFilterFn;
    ErrorFn errorFn;
    VersionFn versionFn;
    IoctlFn ioctlFn;

    static int Main(string[] args)
    {
        if (args != null && args.Length > 0 && args[0] == "--check-pack")
            return CheckPack();
        var host = new J2534Host();
        string line;
        while ((line = Console.ReadLine()) != null)
        {
            // A redirected stdin from PowerShell or .NET can start with a UTF-8 BOM.
            line = line.Trim().TrimStart('﻿');
            if (line.Length == 0) continue;
            if (line == "QUIT")
            {
                host.CloseDevice();
                return 0;
            }
            Console.WriteLine(host.Handle(line));
            Console.Out.Flush();
        }
        host.CloseDevice();
        return 0;
    }

    static int CheckPack()
    {
        IntPtr message = AllocMessage();
        try
        {
            WriteId(message, 0x7E8);
            var packed = new byte[4];
            Marshal.Copy(IntPtr.Add(message, 24), packed, 0, 4);
            uint roundTrip = ReadId(message);
            bool ok = packed[0] == 0x00 && packed[1] == 0x00 && packed[2] == 0x07 && packed[3] == 0xE8 && roundTrip == 0x7E8;
            Console.WriteLine(packed[0].ToString("X2") + " " + packed[1].ToString("X2") + " " + packed[2].ToString("X2") + " " + packed[3].ToString("X2"));
            return ok ? 0 : 1;
        }
        finally
        {
            Marshal.FreeHGlobal(message);
        }
    }

    string Handle(string line)
    {
        try
        {
            string verb;
            string rest;
            Split(line, out verb, out rest);
            if (verb == "OPEN") return Open(rest);
            if (verb == "CONNECT") return Connect(rest);
            if (verb == "INIT") return Init(rest);
            if (verb == "FILTER") return Filter(rest);
            if (verb == "READ") return Read(rest);
            if (verb == "WRITE") return Write(rest);
            if (verb == "IOCTL") return Ioctl(rest);
            if (verb == "VERSION") return Version();
            if (verb == "CLOSE")
            {
                CloseDevice();
                return Ok();
            }
            return Fail("Unknown command.", -1);
        }
        catch (Exception ex)
        {
            return Fail(ex.Message, -1);
        }
    }

    string Open(string path)
    {
        CloseDevice();
        if (path.Length >= 2 && path[0] == '"' && path[path.Length - 1] == '"')
            path = path.Substring(1, path.Length - 2);
        if (path.Length == 0 || !File.Exists(path))
            return Fail("Cable library was not found.", -1);
        string folder = Path.GetDirectoryName(path);
        if (!string.IsNullOrEmpty(folder)) SetDllDirectory(folder);
        module = LoadLibrary(path);
        if (module == IntPtr.Zero)
        {
            int win = Marshal.GetLastWin32Error();
            SetDllDirectory(null);
            return Fail("Windows could not load the cable library.", win);
        }
        try
        {
            openFn = (OpenFn)Marshal.GetDelegateForFunctionPointer(Require("PassThruOpen"), typeof(OpenFn));
            closeFn = (CloseFn)Marshal.GetDelegateForFunctionPointer(Require("PassThruClose"), typeof(CloseFn));
            connectFn = (ConnectFn)Marshal.GetDelegateForFunctionPointer(Require("PassThruConnect"), typeof(ConnectFn));
            disconnectFn = (DisconnectFn)Marshal.GetDelegateForFunctionPointer(Require("PassThruDisconnect"), typeof(DisconnectFn));
            readFn = (ReadFn)Marshal.GetDelegateForFunctionPointer(Require("PassThruReadMsgs"), typeof(ReadFn));
            writeFn = (WriteFn)Marshal.GetDelegateForFunctionPointer(Require("PassThruWriteMsgs"), typeof(WriteFn));
            startFilterFn = (StartFilterFn)Marshal.GetDelegateForFunctionPointer(Require("PassThruStartMsgFilter"), typeof(StartFilterFn));
            stopFilterFn = (StopFilterFn)Marshal.GetDelegateForFunctionPointer(Require("PassThruStopMsgFilter"), typeof(StopFilterFn));
            errorFn = (ErrorFn)Marshal.GetDelegateForFunctionPointer(Require("PassThruGetLastError"), typeof(ErrorFn));
            IntPtr versionPtr = GetProcAddress(module, "PassThruReadVersion");
            if (versionPtr != IntPtr.Zero)
                versionFn = (VersionFn)Marshal.GetDelegateForFunctionPointer(versionPtr, typeof(VersionFn));
            IntPtr ioctlPtr = GetProcAddress(module, "PassThruIoctl");
            if (ioctlPtr != IntPtr.Zero)
                ioctlFn = (IoctlFn)Marshal.GetDelegateForFunctionPointer(ioctlPtr, typeof(IoctlFn));
            uint id;
            int status = openFn(IntPtr.Zero, out id);
            if (status != 0)
            {
                string message = Describe(status);
                CloseDevice();
                return Fail(message, status);
            }
            deviceId = id;
            deviceOpen = true;
            return OkHandles();
        }
        catch (Exception ex)
        {
            CloseDevice();
            return Fail(ex.Message, -1);
        }
    }

    string Connect(string rest)
    {
        if (!deviceOpen) return Fail("The cable is not open.", -1);
        string[] parts = Parts(rest);
        if (parts.Length < 2) return Fail("CONNECT needs a bitrate and an 11-bit or 29-bit flag.", -1);
        uint baud;
        if (!uint.TryParse(parts[0], NumberStyles.Integer, CultureInfo.InvariantCulture, out baud) || baud < 1 || baud > 1000000)
            return Fail("Bitrate must be between 1 and 1000000.", -1);
        if (parts[1] != "0" && parts[1] != "1")
            return Fail("The identifier flag must be 0 for 11-bit or 1 for 29-bit.", -1);
        uint protocolId = ProtocolCan;
        if (parts.Length >= 3)
        {
            if (!uint.TryParse(parts[2], NumberStyles.Integer, CultureInfo.InvariantCulture, out protocolId)
                || (protocolId != ProtocolVpw && protocolId != ProtocolKLine && protocolId != ProtocolCan
                    && protocolId != ProtocolIso15765))
                return Fail("Protocol must be 1 (J1850 VPW), 4 (ISO 14230), 5 (CAN), or 6 (ISO 15765).", -1);
        }
        uint flags = 0;
        if (parts.Length >= 4)
        {
            if (!uint.TryParse(parts[3], NumberStyles.HexNumber, CultureInfo.InvariantCulture, out flags))
                return Fail("CONNECT flags must be hexadecimal.", -1);
        }
        DropChannel();
        extended = (protocolId == ProtocolCan || protocolId == ProtocolIso15765) && parts[1] == "1";
        if (extended) flags |= Can29Bit;
        protocol = protocolId;
        wireFlags = flags;
        uint id;
        int status = connectFn(deviceId, protocolId, flags, baud, out id);
        if (status != 0) return Fail(Describe(status), status);
        channelId = id;
        channelOpen = true;
        return OkHandles();
    }

    string Ioctl(string rest)
    {
        if (!channelOpen) return Fail("The channel is not open.", -1);
        if (ioctlFn == null) return Fail("The cable library is missing PassThruIoctl.", -1);
        string[] parts = Parts(rest);
        if (parts.Length < 1) return Fail("IOCTL needs an ioctl id.", -1);
        uint ioctlId = ParseHex(parts[0]);
        if (ioctlId == ReadVbatt)
        {
            IntPtr output = Marshal.AllocHGlobal(4);
            try
            {
                int status = ioctlFn(channelId, ReadVbatt, IntPtr.Zero, output);
                if (status != 0) return Fail(Describe(status), status);
                uint mv = unchecked((uint)Marshal.ReadInt32(output));
                var bytes = BitConverter.GetBytes(mv);
                return "{\"ok\":true,\"data\":\"" + ToHex(bytes) + "\"}";
            }
            finally
            {
                Marshal.FreeHGlobal(output);
            }
        }
        if (ioctlId == FastInit)
        {
            string hex = parts.Length >= 2 ? parts[1].Replace(" ", "") : "";
            return Init(hex);
        }
        if (ioctlId == ClearRxBuffer)
        {
            int status = ioctlFn(channelId, ClearRxBuffer, IntPtr.Zero, IntPtr.Zero);
            if (status != 0) return Fail(Describe(status), status);
            return Ok();
        }
        return Fail("Unsupported IOCTL id for host bridge.", -1);
    }

    string Init(string rest)
    {
        if (!channelOpen) return Fail("The channel is not open.", -1);
        if (protocol != ProtocolKLine) return Fail("Fast init is only used on ISO 14230.", -1);
        if (ioctlFn == null) return Fail("The cable library is missing PassThruIoctl.", -1);
        string hex = rest == null ? "" : rest.Replace(" ", "");
        byte[] request = hex.Length == 0 ? StartCommunication() : ParseHexBytes(hex, 260);
        if (request.Length == 0) return Fail("Fast init needs a start-communication message.", -1);
        IntPtr input = AllocMessage();
        IntPtr output = AllocMessage();
        try
        {
            WriteUInt(input, 0, ProtocolKLine);
            WriteUInt(input, 16, (uint)request.Length);
            Marshal.Copy(request, 0, IntPtr.Add(input, 24), request.Length);
            WriteUInt(output, 0, ProtocolKLine);
            WriteUInt(output, 16, 4128);
            int status = ioctlFn(channelId, FastInit, input, output);
            if (status != 0) return Fail(Describe(status), status);
            uint dataSize = ReadUInt(output, 16);
            if (dataSize > 4128) dataSize = 0;
            var response = new byte[dataSize];
            if (dataSize > 0) Marshal.Copy(IntPtr.Add(output, 24), response, 0, (int)dataSize);
            return "{\"ok\":true,\"data\":\"" + ToHex(response) + "\"}";
        }
        finally
        {
            Marshal.FreeHGlobal(input);
            Marshal.FreeHGlobal(output);
        }
    }

    static byte[] StartCommunication()
    {
        var frame = new byte[] { 0x81, 0x10, 0xF1, 0x81, 0x00 };
        frame[4] = (byte)((frame[0] + frame[1] + frame[2] + frame[3]) & 0xFF);
        return frame;
    }

    string Version()
    {
        if (!deviceOpen || versionFn == null) return Fail("The cable is not open.", -1);
        var firmware = new StringBuilder(80);
        var dll = new StringBuilder(80);
        var api = new StringBuilder(80);
        int status = versionFn(deviceId, firmware, dll, api);
        if (status != 0) return Fail(Describe(status), status);
        return "{\"ok\":true,\"device_id\":" + deviceId.ToString(CultureInfo.InvariantCulture)
            + ",\"channel_id\":" + (channelOpen ? channelId.ToString(CultureInfo.InvariantCulture) : "0")
            + ",\"firmware\":" + Quote(firmware.ToString())
            + ",\"dll\":" + Quote(dll.ToString())
            + ",\"api\":" + Quote(api.ToString()) + "}";
    }

    bool IsCan
    {
        get { return protocol == ProtocolCan || protocol == ProtocolIso15765; }
    }

    string Filter(string rest)
    {
        if (!channelOpen) return Fail("The channel is not open.", -1);
        string[] parts = Parts(rest);
        if (!IsCan) return FilterRaw(parts);
        if (parts.Length < 2) return Fail("FILTER needs an id and a mask.", -1);
        uint id = ParseHex(parts[0]);
        uint mask = ParseHex(parts[1]);
        uint cap = extended ? 0x1FFFFFFFu : 0x7FFu;
        if (id > cap || mask > cap)
            return Fail("The acceptance filter does not fit the selected identifier size.", -1);
        bool isoTp = protocol == ProtocolIso15765;
        uint flowId = 0;
        if (isoTp)
        {
            if (parts.Length < 3) return Fail("An ISO 15765 filter needs the response id, mask, and request id.", -1);
            flowId = ParseHex(parts[2]);
            if (flowId > cap) return Fail("The request id does not fit the selected identifier size.", -1);
        }
        if (filterOpen)
        {
            int stop = stopFilterFn(channelId, filterId);
            filterOpen = false;
            if (stop != 0) return Fail(Describe(stop), stop);
        }
        IntPtr maskMsg = AllocMessage();
        IntPtr patternMsg = AllocMessage();
        IntPtr flowMsg = isoTp ? AllocMessage() : IntPtr.Zero;
        try
        {
            uint txFlags = (extended ? Can29Bit : 0) | (isoTp ? Iso15765FramePad : 0);
            FillId(maskMsg, protocol, mask, txFlags);
            FillId(patternMsg, protocol, id, txFlags);
            if (isoTp) FillId(flowMsg, protocol, flowId, txFlags);
            uint created;
            int status = startFilterFn(channelId, isoTp ? FlowControlFilter : PassFilter, maskMsg, patternMsg, flowMsg, out created);
            if (status != 0) return Fail(Describe(status), status);
            filterId = created;
            filterOpen = true;
            return Ok();
        }
        finally
        {
            Marshal.FreeHGlobal(maskMsg);
            Marshal.FreeHGlobal(patternMsg);
            if (flowMsg != IntPtr.Zero) Marshal.FreeHGlobal(flowMsg);
        }
    }

    // READ [ms]: how long PassThruReadMsgs may wait for a message (default 0, at most 2000).
    // The XHorse Mini-VCI driver ignores this on an ISO15765 channel and blocks about 5 s when
    // nothing arrives, which is why Wideband uses raw CAN by default.
    static uint ReadTimeout(string rest)
    {
        string[] parts = Parts(rest);
        uint timeout;
        if (parts.Length < 1 || !uint.TryParse(parts[0], NumberStyles.Integer, CultureInfo.InvariantCulture, out timeout))
            return 0;
        return timeout > 2000 ? 2000 : timeout;
    }

    string Read(string rest)
    {
        if (!channelOpen) return Fail("The channel is not open.", -1);
        uint timeout = ReadTimeout(rest);
        if (!IsCan) return ReadRaw(timeout);
        IntPtr message = AllocMessage();
        try
        {
            uint count = 1;
            int status = readFn(channelId, message, ref count, timeout);
            if (status == 0x09 || status == 0x10 || (status == 0 && count == 0))
                return "{\"ok\":true,\"empty\":true}";
            if (status != 0) return Fail(Describe(status), status);
            uint rxStatusBits = ReadUInt(message, 4);
            if (protocol == ProtocolIso15765 && (rxStatusBits & RxIndicationMask) != 0)
                return "{\"ok\":true,\"empty\":true}";
            uint dataSize = ReadUInt(message, 16);
            uint extra = ReadUInt(message, 20);
            uint end = dataSize;
            if (extra >= 4 && extra < end) end = extra;
            uint maxEnd = protocol == ProtocolIso15765 ? (uint)(4 + MaxIsoTpPayload) : 12u;
            if (end < 4 || end > maxEnd)
                return Fail("The cable returned a CAN frame with an unexpected length.", -1);
            if (protocol == ProtocolIso15765 && end == 4)
                return "{\"ok\":true,\"empty\":true}";
            int payloadLength = (int)end - 4;
            var payload = new byte[payloadLength];
            if (payloadLength > 0)
                Marshal.Copy(IntPtr.Add(message, 28), payload, 0, payloadLength);
            uint id = ReadId(message);
            uint rxStatus = ReadUInt(message, 4);
            uint timestamp = ReadUInt(message, 12);
            bool frameExtended = (rxStatus & Can29Bit) != 0;
            return "{\"ok\":true,\"empty\":false,\"id\":\"" + id.ToString("X")
                + "\",\"extended\":" + (frameExtended ? "true" : "false")
                + ",\"data\":\"" + ToHex(payload)
                + "\",\"timestamp\":" + timestamp.ToString(CultureInfo.InvariantCulture) + "}";
        }
        finally
        {
            Marshal.FreeHGlobal(message);
        }
    }

    string Write(string rest)
    {
        if (!channelOpen) return Fail("The channel is not open.", -1);
        if (!IsCan) return WriteRaw(rest);
        string[] parts = Parts(rest);
        if (parts.Length < 1) return Fail("WRITE needs a CAN id.", -1);
        uint id = ParseHex(parts[0]);
        bool isoTp = protocol == ProtocolIso15765;
        byte[] payload = parts.Length > 1 ? ParsePayload(parts[1], isoTp ? MaxIsoTpPayload : 8) : new byte[0];
        if (isoTp && payload.Length == 0) return Fail("An ISO 15765 message needs at least one data byte.", -1);
        if (!extended && id > 0x7FF)
            return Fail("An 11-bit CAN id must be 0x7FF or below.", -1);
        if (extended && id > 0x1FFFFFFF)
            return Fail("A 29-bit CAN id must be 0x1FFFFFFF or below.", -1);
        IntPtr message = AllocMessage();
        try
        {
            uint txFlags = (extended ? Can29Bit : 0) | (isoTp ? Iso15765FramePad : 0);
            WriteUInt(message, 0, protocol);
            WriteUInt(message, 8, txFlags);
            WriteUInt(message, 16, (uint)(4 + payload.Length));
            WriteId(message, id);
            if (payload.Length > 0)
                Marshal.Copy(payload, 0, IntPtr.Add(message, 28), payload.Length);
            uint count = 1;
            int status = writeFn(channelId, message, ref count, 0);
            if (status == 0x09 || status == 0x11 || (status == 0 && count == 0))
            {
                int reported = status == 0 ? 0x11 : status;
                return Busy(Describe(reported), reported);
            }
            if (status != 0) return Fail(Describe(status), status);
            return Ok();
        }
        finally
        {
            Marshal.FreeHGlobal(message);
        }
    }

    void DropChannel()
    {
        if (filterOpen && stopFilterFn != null)
        {
            try { stopFilterFn(channelId, filterId); }
            catch (Exception) { }
            filterOpen = false;
        }
        if (channelOpen && disconnectFn != null)
        {
            try { disconnectFn(channelId); }
            catch (Exception) { }
            channelOpen = false;
        }
    }

    void CloseDevice()
    {
        DropChannel();
        if (deviceOpen && closeFn != null)
        {
            try { closeFn(deviceId); }
            catch (Exception) { }
        }
        deviceOpen = false;
        extended = false;
        protocol = ProtocolCan;
        wireFlags = 0;
        openFn = null;
        closeFn = null;
        connectFn = null;
        disconnectFn = null;
        readFn = null;
        writeFn = null;
        startFilterFn = null;
        stopFilterFn = null;
        errorFn = null;
        versionFn = null;
        ioctlFn = null;
        if (module != IntPtr.Zero)
        {
            FreeLibrary(module);
            module = IntPtr.Zero;
        }
        SetDllDirectory(null);
    }

    IntPtr Require(string name)
    {
        IntPtr address = GetProcAddress(module, name);
        if (address == IntPtr.Zero)
            throw new InvalidOperationException("The cable library is missing " + name + ".");
        return address;
    }

    string Describe(int status)
    {
        if (errorFn != null)
        {
            var buffer = new StringBuilder(256);
            if (errorFn(buffer) == 0)
            {
                string text = buffer.ToString().Trim();
                if (text.Length > 0) return text;
            }
        }
        return StatusName(status);
    }

    static string StatusName(int status)
    {
        switch (status)
        {
            case 0x00: return "STATUS_NOERROR";
            case 0x01: return "ERR_NOT_SUPPORTED";
            case 0x02: return "ERR_INVALID_CHANNEL_ID";
            case 0x03: return "ERR_INVALID_PROTOCOL_ID";
            case 0x04: return "ERR_NULL_PARAMETER";
            case 0x05: return "ERR_INVALID_IOCTL_VALUE";
            case 0x06: return "ERR_INVALID_FLAGS";
            case 0x07: return "ERR_FAILED";
            case 0x08: return "ERR_DEVICE_NOT_CONNECTED";
            case 0x09: return "ERR_TIMEOUT";
            case 0x0A: return "ERR_INVALID_MSG";
            case 0x0B: return "ERR_INVALID_TIME_INTERVAL";
            case 0x0C: return "ERR_EXCEEDED_LIMIT";
            case 0x0D: return "ERR_INVALID_MSG_ID";
            case 0x0E: return "ERR_DEVICE_IN_USE";
            case 0x0F: return "ERR_INVALID_IOCTL_ID";
            case 0x10: return "ERR_BUFFER_EMPTY";
            case 0x11: return "ERR_BUFFER_FULL";
            case 0x12: return "ERR_BUFFER_OVERFLOW";
            case 0x13: return "ERR_PIN_INVALID";
            case 0x14: return "ERR_CHANNEL_IN_USE";
            case 0x15: return "ERR_MSG_PROTOCOL_ID";
            case 0x16: return "ERR_INVALID_FILTER_ID";
            case 0x17: return "ERR_NO_FLOW_CONTROL";
            case 0x18: return "ERR_NOT_UNIQUE";
            case 0x19: return "ERR_INVALID_BAUDRATE";
            case 0x1A: return "ERR_INVALID_DEVICE_ID";
            default: return "The cable library returned status " + status.ToString(CultureInfo.InvariantCulture) + ".";
        }
    }

    string FilterRaw(string[] parts)
    {
        if (parts.Length < 2) return Fail("FILTER needs a mask and a pattern.", -1);
        byte[] mask = ParseHexBytes(parts[0], 12);
        byte[] pattern = ParseHexBytes(parts[1], 12);
        if (mask.Length == 0 || mask.Length != pattern.Length)
            return Fail("The filter mask and pattern must be the same length.", -1);
        if (filterOpen)
        {
            int stop = stopFilterFn(channelId, filterId);
            filterOpen = false;
            if (stop != 0) return Fail(Describe(stop), stop);
        }
        IntPtr maskMsg = AllocMessage();
        IntPtr patternMsg = AllocMessage();
        try
        {
            WriteUInt(maskMsg, 0, protocol);
            WriteUInt(maskMsg, 16, (uint)mask.Length);
            Marshal.Copy(mask, 0, IntPtr.Add(maskMsg, 24), mask.Length);
            WriteUInt(patternMsg, 0, protocol);
            WriteUInt(patternMsg, 16, (uint)pattern.Length);
            Marshal.Copy(pattern, 0, IntPtr.Add(patternMsg, 24), pattern.Length);
            uint created;
            int status = startFilterFn(channelId, PassFilter, maskMsg, patternMsg, IntPtr.Zero, out created);
            if (status != 0) return Fail(Describe(status), status);
            filterId = created;
            filterOpen = true;
            return Ok();
        }
        finally
        {
            Marshal.FreeHGlobal(maskMsg);
            Marshal.FreeHGlobal(patternMsg);
        }
    }

    string ReadRaw(uint timeout)
    {
        IntPtr message = AllocMessage();
        try
        {
            uint count = 1;
            int status = readFn(channelId, message, ref count, timeout);
            if (status == 0x09 || status == 0x10 || (status == 0 && count == 0))
                return "{\"ok\":true,\"empty\":true}";
            if (status != 0) return Fail(Describe(status), status);
            uint dataSize = ReadUInt(message, 16);
            uint extra = ReadUInt(message, 20);
            if (dataSize > 4128) return Fail("The cable returned a frame that is too long.", -1);
            int length = (int)dataSize;
            if (extra > 0 && extra < dataSize) length = (int)extra;
            if (length < 1) return "{\"ok\":true,\"empty\":true}";
            var payload = new byte[length];
            Marshal.Copy(IntPtr.Add(message, 24), payload, 0, length);
            uint timestamp = ReadUInt(message, 12);
            return "{\"ok\":true,\"empty\":false,\"raw\":true,\"data\":\"" + ToHex(payload)
                + "\",\"timestamp\":" + timestamp.ToString(CultureInfo.InvariantCulture) + "}";
        }
        finally
        {
            Marshal.FreeHGlobal(message);
        }
    }

    string WriteRaw(string rest)
    {
        string[] parts = Parts(rest);
        if (parts.Length < 1) return Fail("WRITE needs frame bytes.", -1);
        int max = protocol == ProtocolVpw ? 11 : 260;
        byte[] payload = ParseHexBytes(parts[0], max);
        if (payload.Length == 0) return Fail("WRITE needs frame bytes.", -1);
        IntPtr message = AllocMessage();
        try
        {
            WriteUInt(message, 0, protocol);
            WriteUInt(message, 8, wireFlags);
            WriteUInt(message, 16, (uint)payload.Length);
            Marshal.Copy(payload, 0, IntPtr.Add(message, 24), payload.Length);
            uint count = 1;
            int status = writeFn(channelId, message, ref count, 0);
            if (status == 0x09 || status == 0x11 || (status == 0 && count == 0))
            {
                int reported = status == 0 ? 0x11 : status;
                return Busy(Describe(reported), reported);
            }
            if (status != 0) return Fail(Describe(status), status);
            return Ok();
        }
        finally
        {
            Marshal.FreeHGlobal(message);
        }
    }

    static byte[] ParseHexBytes(string hex, int maxBytes)
    {
        if (hex == null || hex.Length == 0) return new byte[0];
        if ((hex.Length % 2) != 0 || hex.Length / 2 > maxBytes)
            throw new FormatException("The frame hex is not valid.");
        var data = new byte[hex.Length / 2];
        for (int i = 0; i < data.Length; i++)
            data[i] = byte.Parse(hex.Substring(i * 2, 2), NumberStyles.HexNumber, CultureInfo.InvariantCulture);
        return data;
    }

    static void FillId(IntPtr message, uint protocolId, uint id, uint txFlags)
    {
        WriteUInt(message, 0, protocolId);
        WriteUInt(message, 8, txFlags);
        WriteUInt(message, 16, 4);
        WriteId(message, id);
    }

    static IntPtr AllocMessage()
    {
        IntPtr message = Marshal.AllocHGlobal(MessageBytes);
        Marshal.Copy(Zeros, 0, message, MessageBytes);
        return message;
    }

    static void WriteId(IntPtr message, uint id)
    {
        var packed = new byte[4];
        packed[0] = (byte)((id >> 24) & 0xFF);
        packed[1] = (byte)((id >> 16) & 0xFF);
        packed[2] = (byte)((id >> 8) & 0xFF);
        packed[3] = (byte)(id & 0xFF);
        Marshal.Copy(packed, 0, IntPtr.Add(message, 24), 4);
    }

    static uint ReadId(IntPtr message)
    {
        var packed = new byte[4];
        Marshal.Copy(IntPtr.Add(message, 24), packed, 0, 4);
        return ((uint)packed[0] << 24) | ((uint)packed[1] << 16) | ((uint)packed[2] << 8) | packed[3];
    }

    static void WriteUInt(IntPtr message, int offset, uint value)
    {
        Marshal.WriteInt32(message, offset, unchecked((int)value));
    }

    static uint ReadUInt(IntPtr message, int offset)
    {
        return unchecked((uint)Marshal.ReadInt32(message, offset));
    }

    static uint ParseHex(string text)
    {
        if (text == null || text.Length == 0 || text.Length > 8)
            throw new FormatException("CAN id hex is not valid.");
        return uint.Parse(text, NumberStyles.HexNumber, CultureInfo.InvariantCulture);
    }

    static byte[] ParsePayload(string hex, int maxBytes)
    {
        if (hex == null || hex.Length == 0) return new byte[0];
        if ((hex.Length % 2) != 0 || hex.Length > maxBytes * 2)
            throw new FormatException(maxBytes == 8
                ? "A classic CAN frame holds at most 8 data bytes."
                : "The message is longer than the cable accepts.");
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

    static void Split(string line, out string verb, out string rest)
    {
        int space = line.IndexOf(' ');
        if (space < 0)
        {
            verb = line;
            rest = "";
            return;
        }
        verb = line.Substring(0, space);
        rest = line.Substring(space + 1).Trim();
    }

    static string[] Parts(string rest)
    {
        if (rest == null || rest.Length == 0) return new string[0];
        return rest.Split(new char[] { ' ' }, StringSplitOptions.RemoveEmptyEntries);
    }

    static string Ok()
    {
        return "{\"ok\":true}";
    }

    string OkHandles()
    {
        return "{\"ok\":true,\"device_id\":" + deviceId.ToString(CultureInfo.InvariantCulture)
            + ",\"channel_id\":" + (channelOpen ? channelId.ToString(CultureInfo.InvariantCulture) : "0") + "}";
    }

    static string Fail(string error, int status)
    {
        return "{\"ok\":false,\"error\":" + Quote(error) + ",\"status\":" + status.ToString(CultureInfo.InvariantCulture) + "}";
    }

    static string Busy(string error, int status)
    {
        return "{\"ok\":false,\"full\":true,\"error\":" + Quote(error) + ",\"status\":" + status.ToString(CultureInfo.InvariantCulture) + "}";
    }

    static string Quote(string value)
    {
        if (value == null) value = "";
        return "\"" + value.Replace("\\", "\\\\").Replace("\"", "\\\"").Replace("\r", " ").Replace("\n", " ") + "\"";
    }
}
