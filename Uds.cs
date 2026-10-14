using System;

namespace Wideband
{
    // ISO 14229 request/response exchange. Service bytes are assembled here.
    // Key calculation, erase routine ids, and memory contents come from the caller.
    sealed class UdsException : Exception
    {
        public readonly byte NegativeResponse;
        public readonly bool TimedOut;

        public UdsException(string message, byte negative, bool timedOut)
            : base(message)
        {
            NegativeResponse = negative;
            TimedOut = timedOut;
        }
    }

    static class UdsEncoding
    {
        public static bool Fits(long value, int length)
        {
            if (value < 0 || length < 1 || length > 4) return false;
            long max = (1L << (length * 8)) - 1;
            return value <= max;
        }

        public static void WriteBigEndian(byte[] target, int offset, int length, long value)
        {
            for (int i = length - 1; i >= 0; i--)
            {
                target[offset + i] = (byte)(value & 0xFF);
                value >>= 8;
            }
        }

        public static byte Format(int sizeBytes, int addressBytes)
        {
            return (byte)((sizeBytes << 4) | addressBytes);
        }
    }

    sealed class UdsClient
    {
        const byte Negative = 0x7F;
        const byte ResponsePending = 0x78;
        readonly IDiagnosticLink link;
        readonly int maxPending;
        int p2Ms;
        int p2StarMs;

        public UdsClient(IDiagnosticLink link, int maxPending)
        {
            if (link == null) throw new ArgumentNullException("link");
            if (maxPending < 1)
                throw new ArgumentOutOfRangeException("maxPending", "The pending-response limit must be at least 1.");
            this.link = link;
            this.maxPending = maxPending;
            p2Ms = link.TimeoutMs;
            p2StarMs = link.TimeoutMs;
        }

        public static int ExtendedWait(int p2StarMs)
        {
            if (p2StarMs < 5000) return 5000;
            return p2StarMs;
        }

        public void SetTimers(int p2, int p2Star)
        {
            if (p2 > 0) p2Ms = p2;
            if (p2Star > 0) p2StarMs = p2Star;
        }

        public byte[] Request(byte[] request, byte positiveService)
        {
            if (request == null || request.Length == 0) throw new ArgumentException("The diagnostic request is empty.");
            try
            {
                link.Send(request);
            }
            catch (IsoTpException ex)
            {
                throw new UdsException(ex.Message, 0, false);
            }
            byte[] response = Receive();
            if (response == null)
                throw new UdsException("Timed out waiting for service " + request[0].ToString("X2") + ".", 0, true);
            if (response.Length >= 3 && response[0] == Negative)
            {
                if (response[1] != request[0])
                    throw new UdsException("The ECU rejected a different service.", response[2], false);
                throw new UdsException(NegativeText(response[2]), response[2], false);
            }
            if (response.Length < 1 || response[0] != positiveService)
                throw new UdsException("The diagnostic response was not valid.", 0, false);
            return response;
        }

        byte[] Receive()
        {
            int pending = 0;
            int wait = p2Ms;
            while (true)
            {
                if (wait < 1) wait = 1;
                link.SetTimeout(wait);
                byte[] response;
                try
                {
                    if (!link.TryReceive(out response) || response == null)
                        return null;
                }
                catch (IsoTpException ex)
                {
                    throw new UdsException(ex.Message, 0, false);
                }
                if (response.Length >= 3 && response[0] == Negative && response[2] == ResponsePending)
                {
                    pending++;
                    if (pending > maxPending)
                        throw new UdsException("The ECU response is still pending.", ResponsePending, false);
                    wait = ExtendedWait(p2StarMs);
                    continue;
                }
                return response;
            }
        }

        static string NegativeText(byte code)
        {
            switch (code)
            {
                case 0x13: return "The message length is wrong.";
                case 0x22: return "Conditions are not correct.";
                case 0x24: return "The request sequence is wrong.";
                case 0x31: return "The request is out of range.";
                case 0x33: return "Security access was denied.";
                case 0x72: return "The transfer failed.";
                case 0x78: return "The ECU response is still pending.";
                case 0x7F: return "The service is not supported in the active session.";
                default: return "The ECU returned negative response " + code.ToString("X2") + ".";
            }
        }
    }

    static class UdsServices
    {
        public static void Present(UdsClient client)
        {
            byte[] response = client.Request(new byte[] { 0x3E, 0x00 }, 0x7E);
            if (response.Length < 2 || response[1] != 0x00)
                throw new UdsException("The tester-present response was not valid.", 0, false);
        }

        public static byte[] Read(UdsClient client, long address, int count, int addressBytes, int sizeBytes)
        {
            if (count < 1) throw new ArgumentOutOfRangeException("count", "A memory read must request at least one byte.");
            if (!UdsEncoding.Fits(address, addressBytes) || !UdsEncoding.Fits(count, sizeBytes))
                throw new ArgumentOutOfRangeException("address", "The read address or length does not fit in its field.");
            byte[] request = new byte[2 + addressBytes + sizeBytes];
            request[0] = 0x23;
            request[1] = UdsEncoding.Format(sizeBytes, addressBytes);
            UdsEncoding.WriteBigEndian(request, 2, addressBytes, address);
            UdsEncoding.WriteBigEndian(request, 2 + addressBytes, sizeBytes, count);
            byte[] response = client.Request(request, 0x63);
            if (response.Length != 1 + count)
                throw new UdsException("The memory read length did not match.", 0, false);
            byte[] data = new byte[count];
            Buffer.BlockCopy(response, 1, data, 0, count);
            return data;
        }

        public static byte[] ReadRange(UdsClient client, long address, int size, int addressBytes, int sizeBytes, int chunkBytes, Action<int, int> progress)
        {
            if (size < 1) throw new ArgumentOutOfRangeException("size", "A memory read must request at least one byte.");
            if (chunkBytes < 1) throw new ArgumentOutOfRangeException("chunkBytes", "A memory read chunk must hold at least one byte.");
            if (!UdsEncoding.Fits(chunkBytes, sizeBytes))
                throw new ArgumentOutOfRangeException("chunkBytes", "The read chunk does not fit in the size field.");
            if (!UdsEncoding.Fits(address, addressBytes) || !UdsEncoding.Fits(address + size - 1, addressBytes))
                throw new ArgumentOutOfRangeException("address", "The read range does not fit in the address field.");
            byte[] data = new byte[size];
            int offset = 0;
            while (offset < size)
            {
                int count = chunkBytes;
                if (count > size - offset) count = size - offset;
                byte[] chunk = Read(client, address + offset, count, addressBytes, sizeBytes);
                Buffer.BlockCopy(chunk, 0, data, offset, count);
                offset += count;
                if (progress != null) progress(offset, size);
            }
            return data;
        }

        public static void StartRoutine(UdsClient client, ushort routineId, long address, int size, int addressBytes, int sizeBytes)
        {
            if (routineId == 0)
                throw new ArgumentOutOfRangeException("routineId", "The routine id comes from the caller and cannot be zero.");
            if (!UdsEncoding.Fits(address, addressBytes) || !UdsEncoding.Fits(size, sizeBytes))
                throw new ArgumentOutOfRangeException("address", "The routine address or length does not fit in its field.");
            byte[] request = new byte[4 + 1 + addressBytes + sizeBytes];
            request[0] = 0x31;
            request[1] = 0x01;
            request[2] = (byte)(routineId >> 8);
            request[3] = (byte)routineId;
            request[4] = UdsEncoding.Format(sizeBytes, addressBytes);
            UdsEncoding.WriteBigEndian(request, 5, addressBytes, address);
            UdsEncoding.WriteBigEndian(request, 5 + addressBytes, sizeBytes, size);
            byte[] response = client.Request(request, 0x71);
            if (response.Length < 4 || response[1] != 0x01 || response[2] != request[2] || response[3] != request[3])
                throw new UdsException("The routine response was not valid.", 0, false);
        }

        public static void HardReset(UdsClient client)
        {
            byte[] response = client.Request(new byte[] { 0x11, 0x01 }, 0x51);
            if (response.Length < 2 || response[1] != 0x01)
                throw new UdsException("The reset response was not valid.", 0, false);
        }
    }

    enum DiagnosticProtocol
    {
        Uds,
        Kwp
    }

    delegate byte[] ComputeSecurityKey(byte[] seed);

    static class DiagnosticService
    {
        public static byte ProgrammingSession(DiagnosticProtocol protocol)
        {
            return protocol == DiagnosticProtocol.Kwp ? (byte)0x85 : (byte)0x02;
        }

        public static byte[] TesterPresentRequest(DiagnosticProtocol protocol)
        {
            if (protocol == DiagnosticProtocol.Kwp) return new byte[] { 0x3E };
            return new byte[] { 0x3E, 0x00 };
        }

        public static void TesterPresent(UdsClient client, DiagnosticProtocol protocol)
        {
            if (client == null) throw new ArgumentNullException("client");
            byte[] response = client.Request(TesterPresentRequest(protocol), 0x7E);
            if (protocol == DiagnosticProtocol.Uds && (response.Length < 2 || response[1] != 0x00))
                throw new UdsException("The tester-present response was not valid.", 0, false);
            if (protocol == DiagnosticProtocol.Kwp && (response.Length < 1 || response[0] != 0x7E))
                throw new UdsException("The tester-present response was not valid.", 0, false);
        }
    }
}
