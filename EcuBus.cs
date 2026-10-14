using System;
using System.Collections.Generic;
using System.Text;

namespace Wideband
{
    // Wire formats under the diagnostic services. Key bytes come only from a registered plugin.
    enum VehicleBus
    {
        CanIsoTp,
        KLine,
        Vpw
    }

    delegate byte[] ComputeKey(byte[] seed, EcuProfile profile);

    interface IRawBus
    {
        bool TryReadRaw(out byte[] data);
        bool TryWriteRaw(byte[] data);
    }

    sealed class EcuProfile
    {
        public const uint IsoNoChecksum = 0x200;

        public string Id = "";
        public string Name = "";
        public string Platform = "";
        public VehicleBus Bus = VehicleBus.CanIsoTp;
        public DiagnosticProtocol Diagnostic = DiagnosticProtocol.Uds;
        public int Bitrate = 500000;
        public uint ProtocolId = 5;
        public uint ConnectFlags = 0;
        public bool ExtendedId = false;
        public uint RequestId = 0x7E0;
        public uint ResponseId = 0x7E8;
        public uint FilterMask = 0x7FF;
        public byte Tester = 0xF1;
        public byte Ecu = 0x10;
        public byte VpwHeader = 0x6C;
        public int MaxChunk = 256;
        public int AddressBytes = 4;
        public int SizeBytes = 4;
        public byte ProgrammingSession = 0x02;
        public byte ReadSession = 0x03;

        public static EcuProfile TacomaCan()
        {
            var profile = new EcuProfile();
            profile.Id = "tacoma-can";
            profile.Name = "Toyota Tacoma 2009+ (Denso CAN)";
            profile.Platform = "Toyota Tacoma";
            profile.Bus = VehicleBus.CanIsoTp;
            profile.Diagnostic = DiagnosticProtocol.Uds;
            profile.Bitrate = 500000;
            profile.ProtocolId = 5;
            profile.RequestId = 0x7E0;
            profile.ResponseId = 0x7E8;
            profile.FilterMask = 0x7FF;
            profile.MaxChunk = 1024;
            profile.ProgrammingSession = 0x02;
            profile.ReadSession = 0x03;
            return profile;
        }

        public static EcuProfile TacomaKLine()
        {
            var profile = new EcuProfile();
            profile.Id = "tacoma-kline";
            profile.Name = "Toyota Tacoma 2005-2008 (KWP2000 K-Line)";
            profile.Platform = "Toyota Tacoma";
            profile.Bus = VehicleBus.KLine;
            profile.Diagnostic = DiagnosticProtocol.Kwp;
            profile.Bitrate = 10400;
            profile.ProtocolId = 4;
            profile.ConnectFlags = IsoNoChecksum;
            profile.MaxChunk = 128;
            profile.ProgrammingSession = 0x85;
            profile.ReadSession = 0x81;
            return profile;
        }

        public static EcuProfile SilveradoCan()
        {
            var profile = new EcuProfile();
            profile.Id = "silverado-can";
            profile.Name = "Chevrolet Silverado 2007+ (Delco CAN)";
            profile.Platform = "Chevrolet Silverado";
            profile.Bus = VehicleBus.CanIsoTp;
            profile.Diagnostic = DiagnosticProtocol.Uds;
            profile.Bitrate = 500000;
            profile.ProtocolId = 5;
            profile.ExtendedId = true;
            profile.RequestId = 0x18DA10F1;
            profile.ResponseId = 0x18DAF110;
            profile.FilterMask = 0x1FFFFFFF;
            profile.MaxChunk = 1024;
            profile.ProgrammingSession = 0x02;
            profile.ReadSession = 0x03;
            return profile;
        }

        public static EcuProfile SilveradoVpw()
        {
            var profile = new EcuProfile();
            profile.Id = "silverado-vpw";
            profile.Name = "Chevrolet Silverado 1999-2007 (J1850 VPW)";
            profile.Platform = "Chevrolet Silverado";
            profile.Bus = VehicleBus.Vpw;
            profile.Diagnostic = DiagnosticProtocol.Uds;
            profile.Bitrate = 10400;
            profile.ProtocolId = 1;
            profile.VpwHeader = 0x6C;
            profile.MaxChunk = 6;
            profile.AddressBytes = 3;
            profile.SizeBytes = 2;
            profile.ProgrammingSession = 0x02;
            profile.ReadSession = 0x03;
            return profile;
        }
    }

    static class SecurityKeys
    {
        static readonly object gate = new object();
        static readonly Dictionary<string, ComputeKey> plugins = new Dictionary<string, ComputeKey>();

        public static void Register(string profileId, ComputeKey compute)
        {
            if (profileId == null || profileId.Length == 0)
                throw new ArgumentException("The profile id is required.");
            if (compute == null) throw new ArgumentNullException("compute");
            lock (gate) plugins[profileId] = compute;
        }

        public static void Clear(string profileId)
        {
            if (profileId == null) return;
            lock (gate) plugins.Remove(profileId);
        }

        public static byte[] Compute(byte[] seed, EcuProfile profile)
        {
            if (profile == null) throw new ArgumentNullException("profile");
            if (seed == null) throw new ArgumentNullException("seed");
            ComputeKey compute;
            lock (gate)
            {
                if (!plugins.TryGetValue(profile.Id, out compute)) compute = null;
            }
            if (compute == null)
                throw new InvalidOperationException("No security key function is configured for " + profile.Name + ".");
            byte[] key = compute(seed, profile);
            if (key == null || key.Length == 0)
                throw new InvalidOperationException("The security key function returned no key.");
            return key;
        }
    }

    static class KwpFrame
    {
        public static byte[] Pack(byte target, byte source, byte[] payload)
        {
            if (payload == null || payload.Length < 1 || payload.Length > 255)
                throw new ArgumentOutOfRangeException("payload", "A KWP2000 message carries 1 to 255 data bytes.");
            bool extra = payload.Length > 63;
            int header = extra ? 4 : 3;
            var frame = new byte[header + payload.Length + 1];
            frame[0] = extra ? (byte)0x80 : (byte)(0x80 | payload.Length);
            frame[1] = target;
            frame[2] = source;
            int at = 3;
            if (extra)
            {
                frame[3] = (byte)payload.Length;
                at = 4;
            }
            Buffer.BlockCopy(payload, 0, frame, at, payload.Length);
            frame[frame.Length - 1] = Checksum(frame, frame.Length - 1);
            return frame;
        }

        public static byte[] TryUnpack(byte[] frame, byte tester)
        {
            if (frame == null || frame.Length < 4) return null;
            int mode = frame[0] & 0xC0;
            if (mode != 0x80 && mode != 0xC0) return null;
            int bits = frame[0] & 0x3F;
            int header;
            int length;
            if (bits == 0)
            {
                if (frame.Length < 5) return null;
                header = 4;
                length = frame[3];
            }
            else
            {
                header = 3;
                length = bits;
            }
            if (length < 1 || frame[1] != tester) return null;
            if (frame.Length == header + length + 1)
            {
                if (Checksum(frame, frame.Length - 1) != frame[frame.Length - 1]) return null;
            }
            else if (frame.Length != header + length)
            {
                return null;
            }
            var payload = new byte[length];
            Buffer.BlockCopy(frame, header, payload, 0, length);
            return payload;
        }

        public static byte Checksum(byte[] frame, int length)
        {
            int sum = 0;
            for (int i = 0; i < length; i++) sum = (sum + frame[i]) & 0xFF;
            return (byte)sum;
        }
    }

    static class VpwFrame
    {
        public static byte[] Pack(byte header, byte target, byte source, byte[] payload)
        {
            if (payload == null || payload.Length < 1 || payload.Length > 8)
                throw new ArgumentOutOfRangeException("payload", "A J1850 VPW message carries 1 to 8 data bytes.");
            var frame = new byte[3 + payload.Length];
            frame[0] = header;
            frame[1] = target;
            frame[2] = source;
            Buffer.BlockCopy(payload, 0, frame, 3, payload.Length);
            return frame;
        }

        public static byte[] TryUnpack(byte[] frame, byte tester)
        {
            if (frame == null || frame.Length < 4 || frame.Length > 11) return null;
            if (frame[1] != tester) return null;
            int length = frame.Length - 3;
            var payload = new byte[length];
            Buffer.BlockCopy(frame, 3, payload, 0, length);
            return payload;
        }
    }

    delegate byte[] FramePack(byte[] payload);
    delegate byte[] FrameUnpack(byte[] frame);

    class FramedLink : IDiagnosticLink
    {
        readonly object gate = new object();
        readonly IRawBus bus;
        readonly IsoTpClock clock;
        readonly FramePack pack;
        readonly FrameUnpack unpack;
        readonly string name;
        int timeoutMs;

        public FramedLink(IRawBus bus, int timeoutMs, IsoTpClock clock, string name, FramePack pack, FrameUnpack unpack)
        {
            if (bus == null) throw new ArgumentNullException("bus");
            if (pack == null) throw new ArgumentNullException("pack");
            if (unpack == null) throw new ArgumentNullException("unpack");
            if (name == null || name.Length == 0) throw new ArgumentException("The bus name is required.");
            if (timeoutMs < 1)
                throw new ArgumentOutOfRangeException("timeoutMs", "The bus timeout must be at least 1 ms.");
            this.bus = bus;
            this.timeoutMs = timeoutMs;
            this.clock = clock == null ? new IsoTpClock() : clock;
            this.name = name;
            this.pack = pack;
            this.unpack = unpack;
        }

        public int TimeoutMs
        {
            get { return timeoutMs; }
        }

        public void SetTimeout(int value)
        {
            if (value < 1)
                throw new ArgumentOutOfRangeException("value", "The bus timeout must be at least 1 ms.");
            timeoutMs = value;
        }

        public void Send(byte[] payload)
        {
            byte[] frame;
            try
            {
                frame = pack(payload);
            }
            catch (Exception ex)
            {
                throw new IsoTpException(ex.Message == null || ex.Message.Length == 0 ? "The message could not be framed." : ex.Message);
            }
            lock (gate)
            {
                int start = clock.Milliseconds;
                while (!bus.TryWriteRaw(frame))
                {
                    if (Since(start) >= timeoutMs)
                        throw new IsoTpException("The " + name + " write timed out.");
                    clock.Wait(1);
                }
            }
        }

        public bool TryReceive(out byte[] payload)
        {
            payload = null;
            lock (gate)
            {
                int start = clock.Milliseconds;
                while (Since(start) < timeoutMs)
                {
                    byte[] frame;
                    if (!bus.TryReadRaw(out frame) || frame == null)
                    {
                        clock.Wait(1);
                        continue;
                    }
                    byte[] parsed = unpack(frame);
                    if (parsed == null) continue;
                    payload = parsed;
                    return true;
                }
            }
            return false;
        }

        int Since(int start)
        {
            return unchecked((int)((uint)clock.Milliseconds - (uint)start));
        }
    }

    sealed class KLineLink : FramedLink
    {
        public KLineLink(IRawBus bus, byte ecu, byte tester, int timeoutMs)
            : this(bus, ecu, tester, timeoutMs, null)
        {
        }

        public KLineLink(IRawBus bus, byte ecu, byte tester, int timeoutMs, IsoTpClock clock)
            : base(bus, timeoutMs, clock, "K-Line",
                delegate (byte[] payload) { return KwpFrame.Pack(ecu, tester, payload); },
                delegate (byte[] frame) { return KwpFrame.TryUnpack(frame, tester); })
        {
        }
    }

    sealed class VpwLink : FramedLink
    {
        public VpwLink(IRawBus bus, byte header, byte ecu, byte tester, int timeoutMs)
            : this(bus, header, ecu, tester, timeoutMs, null)
        {
        }

        public VpwLink(IRawBus bus, byte header, byte ecu, byte tester, int timeoutMs, IsoTpClock clock)
            : base(bus, timeoutMs, clock, "VPW",
                delegate (byte[] payload) { return VpwFrame.Pack(header, ecu, tester, payload); },
                delegate (byte[] frame) { return VpwFrame.TryUnpack(frame, tester); })
        {
        }
    }

    sealed class TracingLink : IDiagnosticLink
    {
        readonly IDiagnosticLink inner;
        readonly Action<string> trace;

        public TracingLink(IDiagnosticLink inner, Action<string> trace)
        {
            if (inner == null) throw new ArgumentNullException("inner");
            this.inner = inner;
            this.trace = trace;
        }

        public int TimeoutMs
        {
            get { return inner.TimeoutMs; }
        }

        public void SetTimeout(int timeoutMs)
        {
            inner.SetTimeout(timeoutMs);
        }

        public void Send(byte[] payload)
        {
            if (trace != null) trace("TX " + Hex(payload));
            inner.Send(payload);
        }

        public bool TryReceive(out byte[] payload)
        {
            bool found = inner.TryReceive(out payload);
            if (trace != null)
            {
                if (found && payload != null) trace("RX " + Hex(payload));
                else trace("RX timeout");
            }
            return found;
        }

        static string Hex(byte[] data)
        {
            if (data == null || data.Length == 0) return "";
            var text = new StringBuilder(data.Length * 2);
            for (int i = 0; i < data.Length; i++) text.Append(data[i].ToString("X2"));
            return text.ToString();
        }
    }
}
