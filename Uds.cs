using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Text;
using System.Threading;

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

    sealed class EcuIdentification
    {
        public string Vin = "";
        public string CalId = "";
        public string Cvn = "";
        public string Details = "";
    }

    static class EcuIdentifier
    {
        public static EcuIdentification Query(IDiagnosticLink link, EcuProfile profile)
        {
            if (link == null) throw new ArgumentNullException("link");
            var id = new EcuIdentification();
            var client = new UdsClient(link, 2);

            id.Vin = QueryVin(client, link, profile);
            id.CalId = QueryCalId(client, link, profile);
            id.Cvn = QueryCvn(client, link, profile);

            var details = new StringBuilder();
            if (id.Vin.Length > 0) details.Append("VIN: ").Append(id.Vin).Append("  ");
            if (id.CalId.Length > 0) details.Append("Cal ID: ").Append(id.CalId).Append("  ");
            if (id.Cvn.Length > 0) details.Append("CVN: ").Append(id.Cvn);
            id.Details = details.ToString().Trim();
            if (id.Details.Length == 0)
                id.Details = "No identification data returned by the ECU.";
            return id;
        }

        static string QueryVin(UdsClient client, IDiagnosticLink link, EcuProfile profile)
        {
            try
            {
                byte[] resp = SendRaw(link, new byte[] { 0x09, 0x02 });
                string vin = ParseVin(resp);
                if (vin.Length > 0) return vin;
            }
            catch { }

            try
            {
                byte[] resp = client.Request(new byte[] { 0x22, 0xF1, 0x90 }, 0x62);
                string vin = ParseVin(resp);
                if (vin.Length > 0) return vin;
            }
            catch { }

            return "";
        }

        static string QueryCalId(UdsClient client, IDiagnosticLink link, EcuProfile profile)
        {
            bool isGm = profile != null && profile.Platform != null && profile.Platform.IndexOf("Silverado", StringComparison.OrdinalIgnoreCase) >= 0;

            try
            {
                byte[] resp = SendRaw(link, new byte[] { 0x09, 0x04 });
                string calId = ParseCalId(resp);
                if (calId.Length > 0) return calId;
            }
            catch { }

            if (isGm)
            {
                try
                {
                    byte[] resp = client.Request(new byte[] { 0x22, 0x00, 0xC1 }, 0x62);
                    string calId = ParseCalId(resp);
                    if (calId.Length > 0) return calId;
                }
                catch { }
            }
            else
            {
                try
                {
                    byte[] resp = client.Request(new byte[] { 0x22, 0xF1, 0x81 }, 0x62);
                    string calId = ParseCalId(resp);
                    if (calId.Length > 0) return calId;
                }
                catch { }
            }

            return "";
        }

        static string QueryCvn(UdsClient client, IDiagnosticLink link, EcuProfile profile)
        {
            try
            {
                byte[] resp = SendRaw(link, new byte[] { 0x09, 0x06 });
                return ParseCvn(resp);
            }
            catch { }

            return "";
        }

        static byte[] SendRaw(IDiagnosticLink link, byte[] req)
        {
            link.SetTimeout(1000);
            link.Send(req);
            byte[] resp;
            if (link.TryReceive(out resp) && resp != null)
                return resp;
            return new byte[0];
        }

        public static string ParseVin(byte[] data)
        {
            if (data == null || data.Length < 17) return "";
            for (int i = 0; i <= data.Length - 17; i++)
            {
                bool ok = true;
                for (int j = 0; j < 17; j++)
                {
                    byte b = data[i + j];
                    bool isAlphanumeric = (b >= '0' && b <= '9') || (b >= 'A' && b <= 'Z');
                    if (!isAlphanumeric) { ok = false; break; }
                }
                if (ok)
                {
                    var sb = new StringBuilder(17);
                    for (int j = 0; j < 17; j++) sb.Append((char)data[i + j]);
                    return sb.ToString();
                }
            }
            return "";
        }

        public static string ParseCalId(byte[] data)
        {
            if (data == null || data.Length < 4) return "";
            int start = 0;
            if (data.Length >= 2 && data[0] == 0x49 && data[1] == 0x04)
            {
                start = (data.Length > 2 && data[2] < 10) ? 3 : 2;
            }
            else if (data.Length >= 3 && data[0] == 0x62)
            {
                start = 3;
            }

            var sb = new StringBuilder();
            for (int i = start; i < data.Length; i++)
            {
                byte b = data[i];
                if ((b >= '0' && b <= '9') || (b >= 'A' && b <= 'Z') || (b >= 'a' && b <= 'z') || b == '-' || b == '_')
                {
                    sb.Append((char)b);
                }
                else if (sb.Length >= 4 && (b == 0 || b == 0xFF || b == ' ' || b == ','))
                {
                    break;
                }
            }
            string result = sb.ToString().Trim();
            if (result.Length >= 4) return result;

            if (data.Length - start >= 4)
            {
                uint num = 0;
                for (int i = 0; i < 4; i++) num = (num << 8) | data[start + i];
                if (num > 100000) return num.ToString(CultureInfo.InvariantCulture);
            }
            return "";
        }

        public static string ParseCvn(byte[] data)
        {
            if (data == null || data.Length < 4) return "";
            int start = 0;
            if (data.Length >= 2 && data[0] == 0x49 && data[1] == 0x06)
            {
                start = (data.Length > 2 && data[2] < 10) ? 3 : 2;
            }
            var cvns = new List<string>();
            for (int i = start; i + 4 <= data.Length; i += 4)
            {
                string hex = data[i].ToString("X2") + data[i + 1].ToString("X2") + data[i + 2].ToString("X2") + data[i + 3].ToString("X2");
                if (hex != "00000000" && hex != "FFFFFFFF") cvns.Add(hex);
            }
            return string.Join(", ", cvns.ToArray());
        }
    }

    static class StockRomMatcher
    {
        public static string FindMatch(string stockFolder, string calId)
        {
            if (calId == null || calId.Trim().Length == 0) return null;
            if (stockFolder == null || stockFolder.Trim().Length == 0 || !Directory.Exists(stockFolder))
                return null;
            string cleanId = calId.Trim().Replace("-", "").Replace("_", "").ToUpperInvariant();
            string rawId = calId.Trim().ToUpperInvariant();

            string[] files = Directory.GetFiles(stockFolder, "*.*", SearchOption.AllDirectories);
            foreach (string file in files)
            {
                if (!file.EndsWith(".bin", StringComparison.OrdinalIgnoreCase) &&
                    !file.EndsWith(".cuw", StringComparison.OrdinalIgnoreCase))
                    continue;

                string name = Path.GetFileNameWithoutExtension(file).ToUpperInvariant();
                string cleanName = name.Replace("-", "").Replace("_", "");
                if (name.IndexOf(rawId, StringComparison.OrdinalIgnoreCase) >= 0 ||
                    cleanName.IndexOf(cleanId, StringComparison.OrdinalIgnoreCase) >= 0)
                {
                    return file;
                }
            }

            foreach (string file in files)
            {
                if (!file.EndsWith(".bin", StringComparison.OrdinalIgnoreCase) &&
                    !file.EndsWith(".cuw", StringComparison.OrdinalIgnoreCase))
                    continue;

                try
                {
                    var fi = new FileInfo(file);
                    if (fi.Length < 32 || fi.Length > 16 * 1024 * 1024) continue;
                    int readLen = (int)Math.Min(fi.Length, 4096L);
                    var buf = new byte[readLen];
                    using (var fs = new FileStream(file, FileMode.Open, FileAccess.Read, FileShare.ReadWrite))
                    {
                        fs.Read(buf, 0, readLen);
                    }
                    string text = Encoding.ASCII.GetString(buf).ToUpperInvariant();
                    if (text.IndexOf(rawId, StringComparison.OrdinalIgnoreCase) >= 0 ||
                        (cleanId.Length >= 6 && text.Replace("-", "").IndexOf(cleanId, StringComparison.OrdinalIgnoreCase) >= 0))
                    {
                        return file;
                    }
                }
                catch { }
            }

            return null;
        }
    }

    sealed class ObdTelemetrySample
    {
        public double Rpm;
        public double MapKpa;
        public double LoadPct;
        public double CoolantC;
        public double TimingAdvance;
        public double IatC;
        public double ThrottlePct;
        public double Lambda = 1.0;
        public double Target = 1.0;
        public DateTime Timestamp = DateTime.Now;

        public double[] ToRow()
        {
            return new double[] { Rpm, MapKpa, LoadPct, CoolantC, TimingAdvance, IatC, ThrottlePct, Lambda, Target };
        }

        public static string[] Headers()
        {
            return new string[] { "rpm", "map", "load", "ect", "advance", "iat", "tps", "lambda", "target" };
        }
    }

    static class ObdPidDecoder
    {
        public static bool PollPid(IDiagnosticLink link, byte pid, out byte[] data)
        {
            data = null;
            try
            {
                link.SetTimeout(180);
                link.Send(new byte[] { 0x01, pid });
                byte[] resp;
                if (!link.TryReceive(out resp) || resp == null || resp.Length < 3) return false;
                for (int i = 0; i <= resp.Length - 3; i++)
                {
                    if (resp[i] == 0x41 && resp[i + 1] == pid)
                    {
                        int len = resp.Length - (i + 2);
                        data = new byte[len];
                        Buffer.BlockCopy(resp, i + 2, data, 0, len);
                        return true;
                    }
                }
            }
            catch { }
            return false;
        }

        public static double DecodeRpm(byte[] d)
        {
            if (d == null || d.Length < 2) return 0;
            return ((d[0] * 256.0) + d[1]) / 4.0;
        }

        public static double DecodeMap(byte[] d)
        {
            if (d == null || d.Length < 1) return 0;
            return d[0];
        }

        public static double DecodePct(byte[] d)
        {
            if (d == null || d.Length < 1) return 0;
            return (d[0] * 100.0) / 255.0;
        }

        public static double DecodeTemp(byte[] d)
        {
            if (d == null || d.Length < 1) return 0;
            return d[0] - 40.0;
        }

        public static double DecodeAdvance(byte[] d)
        {
            if (d == null || d.Length < 1) return 0;
            return (d[0] / 2.0) - 64.0;
        }

        public static double DecodeLambda(byte[] d)
        {
            if (d == null || d.Length < 2) return 1.0;
            double val = ((d[0] * 256.0) + d[1]) / 32768.0;
            return (val > 0.4 && val < 2.5) ? val : 1.0;
        }
    }

    sealed class LiveTelemetryPoller
    {
        readonly object gate = new object();
        Thread thread;
        volatile bool running;
        readonly Action<ObdTelemetrySample> onSample;

        public LiveTelemetryPoller(Action<ObdTelemetrySample> onSample)
        {
            this.onSample = onSample;
        }

        public bool IsRunning
        {
            get { return running; }
        }

        public void Start()
        {
            lock (gate)
            {
                if (running) return;
                running = true;
                thread = new Thread(Worker);
                thread.IsBackground = true;
                thread.Name = "LiveTelemetryPoller";
                thread.Start();
            }
        }

        public void Stop()
        {
            lock (gate)
            {
                running = false;
            }
        }

        void Worker()
        {
            int slowCycle = 0;
            double lastEct = 85.0;
            double lastIat = 25.0;
            double lastTps = 0.0;
            double lastAdvance = 15.0;
            byte lambdaPid = 0x24;
            bool lambdaTested = false;

            while (running)
            {
                if (!VehicleLink.Current.IsConnected)
                {
                    Thread.Sleep(200);
                    continue;
                }

                ObdTelemetrySample sample = null;
                bool ok = VehicleLink.Current.TryRunDiagnostic(delegate(IDiagnosticLink link, EcuProfile profile)
                {
                    sample = new ObdTelemetrySample();
                    byte[] d;

                    if (ObdPidDecoder.PollPid(link, 0x0C, out d)) sample.Rpm = ObdPidDecoder.DecodeRpm(d);
                    if (ObdPidDecoder.PollPid(link, 0x0B, out d)) sample.MapKpa = ObdPidDecoder.DecodeMap(d);
                    if (ObdPidDecoder.PollPid(link, 0x04, out d)) sample.LoadPct = ObdPidDecoder.DecodePct(d);

                    if (!lambdaTested)
                    {
                        if (ObdPidDecoder.PollPid(link, 0x24, out d)) { lambdaPid = 0x24; sample.Lambda = ObdPidDecoder.DecodeLambda(d); lambdaTested = true; }
                        else if (ObdPidDecoder.PollPid(link, 0x34, out d)) { lambdaPid = 0x34; sample.Lambda = ObdPidDecoder.DecodeLambda(d); lambdaTested = true; }
                        else { sample.Lambda = 1.0; }
                    }
                    else
                    {
                        if (ObdPidDecoder.PollPid(link, lambdaPid, out d)) sample.Lambda = ObdPidDecoder.DecodeLambda(d);
                        else sample.Lambda = 1.0;
                    }

                    slowCycle++;
                    if ((slowCycle % 3) == 0)
                    {
                        if (ObdPidDecoder.PollPid(link, 0x11, out d)) lastTps = ObdPidDecoder.DecodePct(d);
                        if (ObdPidDecoder.PollPid(link, 0x0E, out d)) lastAdvance = ObdPidDecoder.DecodeAdvance(d);
                    }
                    if ((slowCycle % 10) == 0)
                    {
                        if (ObdPidDecoder.PollPid(link, 0x05, out d)) lastEct = ObdPidDecoder.DecodeTemp(d);
                        if (ObdPidDecoder.PollPid(link, 0x0F, out d)) lastIat = ObdPidDecoder.DecodeTemp(d);
                    }

                    sample.ThrottlePct = lastTps;
                    sample.TimingAdvance = lastAdvance;
                    sample.CoolantC = lastEct;
                    sample.IatC = lastIat;
                    sample.Target = 1.0;
                    return true;
                }, out ok);

                if (ok && sample != null && running)
                {
                    try { if (onSample != null) onSample(sample); }
                    catch { }
                }

                Thread.Sleep(50);
            }
        }
    }
}
