using System;
using System.Threading;

namespace Wideband
{
    // ISO 15765-2 on a classic 8-byte CAN link. Segmentation only.
    // It does not pick a diagnostic service or program an ECU.
    sealed class IsoTpException : Exception
    {
        public IsoTpException(string message)
            : base(message)
        {
        }
    }

    sealed class IsoTpConfig
    {
        public uint TransmitId = 0x7E0;
        public uint ReceiveId = 0x7E8;
        public bool ExtendedAddress = false;
        public byte TransmitAddress = 0;
        public byte ReceiveAddress = 0;
        public bool PadFrames = true;
        public byte PadByte = 0xCC;
        public int TimeoutMs = 1000;
        public byte BlockSize = 0;
        public byte SeparationTime = 0;
        public int MaxWaits = 5;
        public int MaxPayload = 65536;
        public bool ExtendedId = false;
    }

    interface IDiagnosticLink
    {
        int TimeoutMs { get; }
        void SetTimeout(int timeoutMs);
        void Send(byte[] payload);
        bool TryReceive(out byte[] payload);
    }

    // Millisecond clock. Sub-millisecond STmin is rounded up to 1 ms.
    class IsoTpClock
    {
        public virtual int Milliseconds
        {
            get { return Environment.TickCount; }
        }

        public virtual void Wait(int milliseconds)
        {
            if (milliseconds > 0) Thread.Sleep(milliseconds);
        }
    }

    static class IsoTpTiming
    {
        public static byte EncodeSeparation(int microseconds)
        {
            if (microseconds <= 0) return 0;
            if (microseconds < 1000)
            {
                int steps = (microseconds + 99) / 100;
                if (steps < 1) steps = 1;
                if (steps > 9) return 1;
                return (byte)(0xF0 + steps);
            }
            int milliseconds = (microseconds + 999) / 1000;
            if (milliseconds > 127) milliseconds = 127;
            return (byte)milliseconds;
        }

        // Reserved STmin values use the longest defined gap, 127 ms.
        public static int SeparationWaitMs(byte stmin)
        {
            if (stmin <= 0x7F) return stmin;
            if (stmin >= 0xF1 && stmin <= 0xF9) return 1;
            return 127;
        }
    }

    sealed class IsoTpLink : IDiagnosticLink
    {
        readonly object gate = new object();
        readonly ICanLink channel;
        readonly IsoTpConfig config;
        readonly IsoTpClock clock;

        public IsoTpLink(ICanLink channel, IsoTpConfig config)
            : this(channel, config, null)
        {
        }

        public IsoTpLink(ICanLink channel, IsoTpConfig config, IsoTpClock clock)
        {
            if (channel == null) throw new ArgumentNullException("channel");
            if (config == null) throw new ArgumentNullException("config");
            if (config.TimeoutMs < 1)
                throw new ArgumentOutOfRangeException("config", "ISO-TP timeout must be at least 1 ms.");
            if (config.MaxPayload < 8)
                throw new ArgumentOutOfRangeException("config", "ISO-TP maximum payload must be at least 8 bytes.");
            if (config.MaxWaits < 0)
                throw new ArgumentOutOfRangeException("config", "ISO-TP wait limit cannot be negative.");
            this.channel = channel;
            this.config = config;
            this.clock = clock == null ? new IsoTpClock() : clock;
        }

        public int TimeoutMs
        {
            get { return config.TimeoutMs; }
        }

        public void SetTimeout(int timeoutMs)
        {
            if (timeoutMs < 1)
                throw new ArgumentOutOfRangeException("timeoutMs", "ISO-TP timeout must be at least 1 ms.");
            config.TimeoutMs = timeoutMs;
        }

        public void Send(byte[] payload)
        {
            if (payload == null) throw new ArgumentNullException("payload");
            if (payload.Length == 0) throw new ArgumentException("ISO-TP cannot send an empty payload.");
            if (payload.Length > config.MaxPayload) throw new ArgumentException("ISO-TP payload is too large.");
            lock (gate)
            {
                if (payload.Length <= SingleLimit())
                {
                    WriteFrame(SingleFrame(payload));
                    return;
                }
                int offset = WriteFirst(payload);
                int sequence = 1;
                int waits = 0;
                while (offset < payload.Length)
                {
                    byte flow;
                    byte block;
                    byte separation;
                    if (!ReadFlow(out flow, out block, out separation))
                        throw new IsoTpException("ISO-TP flow control timed out.");
                    if (flow == 2) throw new IsoTpException("ISO-TP receiver reported overflow.");
                    if (flow == 1)
                    {
                        waits++;
                        if (waits > config.MaxWaits)
                            throw new IsoTpException("ISO-TP flow control wait limit was reached.");
                        continue;
                    }
                    if (flow != 0) throw new IsoTpException("ISO-TP flow control status is not valid.");
                    waits = 0;
                    int sentInBlock = 0;
                    while (offset < payload.Length)
                    {
                        if (block != 0 && sentInBlock >= block) break;
                        if (sentInBlock > 0)
                        {
                            int gap = IsoTpTiming.SeparationWaitMs(separation);
                            if (gap > 0) clock.Wait(gap);
                        }
                        int chunk = ConsecutiveLimit();
                        int remaining = payload.Length - offset;
                        if (chunk > remaining) chunk = remaining;
                        WriteFrame(ConsecutiveFrame(sequence, payload, offset, chunk));
                        offset += chunk;
                        sequence = (sequence + 1) & 0x0F;
                        sentInBlock++;
                    }
                }
            }
        }

        public byte[] Receive()
        {
            byte[] payload;
            if (!TryReceive(out payload))
                throw new IsoTpException("ISO-TP receive timed out.");
            return payload;
        }

        public bool TryReceive(out byte[] payload)
        {
            payload = null;
            lock (gate)
            {
                int start = clock.Milliseconds;
                byte[] message = null;
                int expected = 0;
                int filled = 0;
                int sequence = 1;
                int inBlock = 0;
                while (Since(start, clock.Milliseconds) < config.TimeoutMs)
                {
                    CanFrame frame;
                    if (!ReadOne(out frame))
                    {
                        clock.Wait(1);
                        continue;
                    }
                    if (frame.Data == null || frame.Id != config.ReceiveId) continue;
                    int pci = PciIndex(frame.Data);
                    if (pci < 0 || pci >= frame.Data.Length) continue;
                    int type = (frame.Data[pci] >> 4) & 0x0F;
                    if (type == 0)
                    {
                        byte[] single = TrySingle(frame.Data, pci);
                        if (single == null) continue;
                        payload = single;
                        return true;
                    }
                    if (type == 1)
                    {
                        int length;
                        int copied;
                        if (!TryFirst(frame.Data, pci, out length, out copied, out message))
                            continue;
                        WriteFrame(FlowControl(0));
                        expected = length;
                        filled = copied;
                        sequence = 1;
                        inBlock = 0;
                        start = clock.Milliseconds;
                        continue;
                    }
                    if (type != 2 || message == null) continue;
                    if ((frame.Data[pci] & 0x0F) != sequence)
                        throw new IsoTpException("ISO-TP consecutive frame is out of sequence.");
                    int room = expected - filled;
                    int take = room < ConsecutiveLimit() ? room : ConsecutiveLimit();
                    int available = frame.Data.Length - (pci + 1);
                    if (available < take)
                        throw new IsoTpException("ISO-TP consecutive frame is short.");
                    Buffer.BlockCopy(frame.Data, pci + 1, message, filled, take);
                    filled += take;
                    sequence = (sequence + 1) & 0x0F;
                    inBlock++;
                    start = clock.Milliseconds;
                    if (filled >= expected)
                    {
                        payload = message;
                        return true;
                    }
                    if (config.BlockSize != 0 && inBlock >= config.BlockSize)
                    {
                        WriteFrame(FlowControl(0));
                        inBlock = 0;
                        start = clock.Milliseconds;
                    }
                }
                if (message != null)
                    throw new IsoTpException("ISO-TP consecutive frame timed out.");
                return false;
            }
        }

        int SingleLimit()
        {
            return config.ExtendedAddress ? 6 : 7;
        }

        int ConsecutiveLimit()
        {
            return config.ExtendedAddress ? 6 : 7;
        }

        byte[] SingleFrame(byte[] payload)
        {
            int pci = config.ExtendedAddress ? 1 : 0;
            byte[] data = Blank(pci + 1 + payload.Length);
            data[pci] = (byte)payload.Length;
            Buffer.BlockCopy(payload, 0, data, pci + 1, payload.Length);
            return data;
        }

        int WriteFirst(byte[] payload)
        {
            int pci = config.ExtendedAddress ? 1 : 0;
            bool wide = payload.Length > 4095;
            int start = pci + (wide ? 6 : 2);
            int chunk = 8 - start;
            byte[] data = Blank(8);
            if (!wide)
            {
                data[pci] = (byte)(0x10 | ((payload.Length >> 8) & 0x0F));
                data[pci + 1] = (byte)payload.Length;
            }
            else
            {
                data[pci] = 0x10;
                data[pci + 1] = 0x00;
                data[pci + 2] = (byte)((payload.Length >> 24) & 0xFF);
                data[pci + 3] = (byte)((payload.Length >> 16) & 0xFF);
                data[pci + 4] = (byte)((payload.Length >> 8) & 0xFF);
                data[pci + 5] = (byte)(payload.Length & 0xFF);
            }
            Buffer.BlockCopy(payload, 0, data, start, chunk);
            WriteFrame(data);
            return chunk;
        }

        byte[] ConsecutiveFrame(int sequence, byte[] payload, int offset, int chunk)
        {
            int pci = config.ExtendedAddress ? 1 : 0;
            byte[] data = Blank(pci + 1 + chunk);
            data[pci] = (byte)(0x20 | (sequence & 0x0F));
            Buffer.BlockCopy(payload, offset, data, pci + 1, chunk);
            return data;
        }

        byte[] FlowControl(byte flowStatus)
        {
            int pci = config.ExtendedAddress ? 1 : 0;
            byte[] data = Blank(pci + 3);
            data[pci] = (byte)(0x30 | (flowStatus & 0x0F));
            data[pci + 1] = config.BlockSize;
            data[pci + 2] = config.SeparationTime;
            return data;
        }

        byte[] Blank(int used)
        {
            int length = config.PadFrames ? 8 : used;
            if (length < used) length = used;
            if (length > 8) throw new IsoTpException("ISO-TP frame does not fit in classic CAN.");
            byte[] data = new byte[length];
            if (config.PadFrames)
            {
                for (int i = 0; i < data.Length; i++) data[i] = config.PadByte;
            }
            if (config.ExtendedAddress) data[0] = config.TransmitAddress;
            return data;
        }

        byte[] TrySingle(byte[] data, int pci)
        {
            int length = data[pci] & 0x0F;
            if (length < 1 || length > SingleLimit()) return null;
            if (data.Length < pci + 1 + length) return null;
            byte[] payload = new byte[length];
            Buffer.BlockCopy(data, pci + 1, payload, 0, length);
            return payload;
        }

        bool TryFirst(byte[] data, int pci, out int length, out int copied, out byte[] message)
        {
            length = 0;
            copied = 0;
            message = null;
            if (data.Length < pci + 2) return false;
            int dl12 = ((data[pci] & 0x0F) << 8) | data[pci + 1];
            int start;
            if (dl12 == 0)
            {
                if (data.Length < pci + 6) return false;
                uint wide = ((uint)data[pci + 2] << 24)
                    | ((uint)data[pci + 3] << 16)
                    | ((uint)data[pci + 4] << 8)
                    | data[pci + 5];
                if (wide <= 4095) return false;
                if (wide > (uint)config.MaxPayload)
                    throw new IsoTpException("ISO-TP message is larger than the configured maximum.");
                length = (int)wide;
                start = pci + 6;
            }
            else
            {
                if (dl12 <= SingleLimit()) return false;
                if (dl12 > config.MaxPayload)
                    throw new IsoTpException("ISO-TP message is larger than the configured maximum.");
                length = dl12;
                start = pci + 2;
            }
            int chunk = data.Length - start;
            if (chunk > 8 - start) chunk = 8 - start;
            if (chunk <= 0 || chunk >= length) return false;
            message = new byte[length];
            Buffer.BlockCopy(data, start, message, 0, chunk);
            copied = chunk;
            return true;
        }

        int PciIndex(byte[] data)
        {
            if (!config.ExtendedAddress) return 0;
            if (data.Length < 2 || data[0] != config.ReceiveAddress) return -1;
            return 1;
        }

        bool ReadFlow(out byte flow, out byte block, out byte separation)
        {
            flow = 0;
            block = 0;
            separation = 0;
            int start = clock.Milliseconds;
            while (Since(start, clock.Milliseconds) < config.TimeoutMs)
            {
                CanFrame frame;
                if (!ReadOne(out frame))
                {
                    clock.Wait(1);
                    continue;
                }
                if (frame.Data == null || frame.Id != config.ReceiveId) continue;
                int pci = PciIndex(frame.Data);
                if (pci < 0 || frame.Data.Length < pci + 3) continue;
                if (((frame.Data[pci] >> 4) & 0x0F) != 3) continue;
                flow = (byte)(frame.Data[pci] & 0x0F);
                block = frame.Data[pci + 1];
                separation = frame.Data[pci + 2];
                return true;
            }
            return false;
        }

        bool ReadOne(out CanFrame frame)
        {
            frame = null;
            CanFrame read;
            if (!channel.TryRead(out read) || read == null) return false;
            frame = read;
            return true;
        }

        void WriteFrame(byte[] data)
        {
            int start = clock.Milliseconds;
            var frame = new CanFrame();
            frame.Id = config.TransmitId;
            frame.Extended = config.ExtendedId;
            frame.Data = data;
            while (!channel.TryWrite(frame))
            {
                if (Since(start, clock.Milliseconds) >= config.TimeoutMs)
                    throw new IsoTpException("ISO-TP could not queue a CAN frame.");
                clock.Wait(1);
            }
        }

        static int Since(int start, int now)
        {
            return unchecked((int)((uint)now - (uint)start));
        }
    }
}
