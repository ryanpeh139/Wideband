using System;

namespace Wideband
{
    // ISO 14229 session and security-access sequencing.
    // Key bytes come only from ISecurityKey. This file does not calculate an ECU key.
    enum DiagnosticStep
    {
        Idle,
        RequestExtendedSession,
        WaitExtendedSession,
        RequestSeed,
        WaitSeed,
        CalculateKey,
        SendKey,
        WaitKey,
        Unlocked,
        Failed
    }

    interface ISecurityKey
    {
        byte[] Calculate(byte[] seed);
    }

    sealed class DiagnosticAccessResult
    {
        public DiagnosticStep Step;
        public DiagnosticStep FailedDuring;
        public bool Unlocked;
        public byte[] Seed = new byte[0];
        public byte NegativeResponse;
        public string Error = "";
        public int P2Ms;
        public int P2StarMs;
    }

    sealed class ExtendedSessionAccess
    {
        const byte Negative = 0x7F;
        const byte ResponsePending = 0x78;
        readonly IDiagnosticLink link;
        readonly ISecurityKey keys;
        readonly byte level;
        readonly byte sessionType;
        readonly int maxPending;
        DiagnosticStep step = DiagnosticStep.Idle;
        DiagnosticStep failedDuring = DiagnosticStep.Idle;
        byte[] seed = new byte[0];
        byte[] keyRequest;
        byte negative;
        string error = "";
        bool unlocked;
        int p2Ms;
        int p2StarMs;

        public ExtendedSessionAccess(IDiagnosticLink link, ISecurityKey keys)
            : this(link, keys, 0x01, 10)
        {
        }

        public ExtendedSessionAccess(IDiagnosticLink link, ISecurityKey keys, byte level, int maxPending)
            : this(link, keys, level, maxPending, 0x03)
        {
        }

        public ExtendedSessionAccess(IDiagnosticLink link, ISecurityKey keys, byte level, int maxPending, byte sessionType)
        {
            if (link == null) throw new ArgumentNullException("link");
            if (keys == null) throw new ArgumentNullException("keys");
            if (maxPending < 1)
                throw new ArgumentOutOfRangeException("maxPending", "The pending-response limit must be at least 1.");
            if (level < 0x01 || level > 0x7D || (level % 2) == 0)
                throw new ArgumentOutOfRangeException("level", "The security level must be an odd value from 0x01 through 0x7D.");
            if (!IsSessionType(sessionType))
                throw new ArgumentOutOfRangeException("sessionType", "The diagnostic session must be a UDS session or a KWP2000 session.");
            this.link = link;
            this.keys = keys;
            this.level = level;
            this.maxPending = maxPending;
            this.sessionType = sessionType;
            p2Ms = link.TimeoutMs;
            p2StarMs = link.TimeoutMs;
        }

        public DiagnosticStep Step
        {
            get { return step; }
        }

        public DiagnosticAccessResult Run()
        {
            if (step != DiagnosticStep.Idle && step != DiagnosticStep.Failed && step != DiagnosticStep.Unlocked)
                throw new InvalidOperationException("Security access is already running.");
            step = DiagnosticStep.RequestExtendedSession;
            failedDuring = DiagnosticStep.Idle;
            seed = new byte[0];
            keyRequest = null;
            negative = 0;
            error = "";
            unlocked = false;
            p2Ms = link.TimeoutMs;
            p2StarMs = link.TimeoutMs;
            while (step != DiagnosticStep.Unlocked && step != DiagnosticStep.Failed)
                Advance();
            return Snapshot();
        }

        void Advance()
        {
            switch (step)
            {
                case DiagnosticStep.RequestExtendedSession:
                    if (Send(new byte[] { 0x10, sessionType }))
                        step = DiagnosticStep.WaitExtendedSession;
                    break;
                case DiagnosticStep.WaitExtendedSession:
                    OnSession(Receive());
                    break;
                case DiagnosticStep.RequestSeed:
                    if (Send(new byte[] { 0x27, level }))
                        step = DiagnosticStep.WaitSeed;
                    break;
                case DiagnosticStep.WaitSeed:
                    OnSeed(Receive());
                    break;
                case DiagnosticStep.CalculateKey:
                    OnCalculate();
                    break;
                case DiagnosticStep.SendKey:
                    if (Send(keyRequest))
                        step = DiagnosticStep.WaitKey;
                    break;
                case DiagnosticStep.WaitKey:
                    OnKey(Receive());
                    break;
                default:
                    Fail("Security access is not running.");
                    break;
            }
        }

        void OnSession(byte[] response)
        {
            if (step == DiagnosticStep.Failed) return;
            if (response == null)
            {
                Fail("Timed out waiting for the diagnostic session.");
                return;
            }
            if (response.Length >= 3 && response[0] == Negative)
            {
                FailNegative(response, 0x10);
                return;
            }
            if (response.Length < 2 || response[0] != 0x50 || response[1] != sessionType)
            {
                Fail("The diagnostic session response was not valid.");
                return;
            }
            if (response.Length >= 6)
            {
                int p2 = (response[2] << 8) | response[3];
                int p2Star = ((response[4] << 8) | response[5]) * 10;
                if (p2 > 0) p2Ms = p2;
                if (p2Star > 0) p2StarMs = p2Star;
            }
            step = DiagnosticStep.RequestSeed;
        }

        void OnSeed(byte[] response)
        {
            if (step == DiagnosticStep.Failed) return;
            if (response == null)
            {
                Fail("Timed out waiting for the security seed.");
                return;
            }
            if (response.Length >= 3 && response[0] == Negative)
            {
                FailNegative(response, 0x27);
                return;
            }
            if (response.Length < 3 || response[0] != 0x67 || response[1] != level)
            {
                Fail("The security seed response was not valid.");
                return;
            }
            seed = new byte[response.Length - 2];
            Buffer.BlockCopy(response, 2, seed, 0, seed.Length);
            if (AllZero(seed))
            {
                unlocked = true;
                step = DiagnosticStep.Unlocked;
                return;
            }
            step = DiagnosticStep.CalculateKey;
        }

        void OnCalculate()
        {
            byte[] key;
            try
            {
                key = keys.Calculate((byte[])seed.Clone());
            }
            catch (Exception ex)
            {
                Fail(ex.Message == null || ex.Message.Length == 0 ? "The key calculator failed." : ex.Message);
                return;
            }
            if (key == null || key.Length == 0)
            {
                Fail("The key calculator returned no key.");
                return;
            }
            keyRequest = new byte[2 + key.Length];
            keyRequest[0] = 0x27;
            keyRequest[1] = (byte)(level + 1);
            Buffer.BlockCopy(key, 0, keyRequest, 2, key.Length);
            step = DiagnosticStep.SendKey;
        }

        void OnKey(byte[] response)
        {
            if (step == DiagnosticStep.Failed) return;
            if (response == null)
            {
                Fail("Timed out waiting for the key response.");
                return;
            }
            if (response.Length >= 3 && response[0] == Negative)
            {
                FailNegative(response, 0x27);
                return;
            }
            byte keyLevel = (byte)(level + 1);
            if (response.Length < 2 || response[0] != 0x67 || response[1] != keyLevel)
            {
                Fail("The key response was not valid.");
                return;
            }
            unlocked = true;
            step = DiagnosticStep.Unlocked;
        }

        bool Send(byte[] payload)
        {
            try
            {
                link.Send(payload);
                return true;
            }
            catch (IsoTpException ex)
            {
                Fail(ex.Message);
                return false;
            }
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
                    Fail(ex.Message);
                    return null;
                }
                if (response.Length >= 3 && response[0] == Negative && response[2] == ResponsePending)
                {
                    pending++;
                    if (pending > maxPending) return response;
                    wait = UdsClient.ExtendedWait(p2StarMs);
                    continue;
                }
                return response;
            }
        }

        void FailNegative(byte[] response, byte expectedService)
        {
            byte code = response[2];
            if (response[1] != expectedService)
            {
                Fail("The ECU rejected a different service.");
                return;
            }
            negative = code;
            Fail(NegativeText(code));
        }

        void Fail(string message)
        {
            if (step == DiagnosticStep.Failed) return;
            failedDuring = step;
            error = message;
            unlocked = false;
            step = DiagnosticStep.Failed;
        }

        DiagnosticAccessResult Snapshot()
        {
            var result = new DiagnosticAccessResult();
            result.Step = step;
            result.FailedDuring = failedDuring;
            result.Unlocked = unlocked;
            result.Seed = (byte[])seed.Clone();
            result.NegativeResponse = negative;
            result.Error = error == null ? "" : error;
            result.P2Ms = p2Ms;
            result.P2StarMs = p2StarMs;
            return result;
        }

        public static bool IsSessionType(byte sessionType)
        {
            if (sessionType >= 0x01 && sessionType <= 0x04) return true;
            return sessionType == 0x81 || sessionType == 0x85 || sessionType == 0x86 || sessionType == 0x89;
        }

        static bool AllZero(byte[] value)
        {
            for (int i = 0; i < value.Length; i++)
                if (value[i] != 0) return false;
            return true;
        }

        static string NegativeText(byte code)
        {
            switch (code)
            {
                case 0x12: return "The sub-function is not supported.";
                case 0x13: return "The message length is wrong.";
                case 0x22: return "Conditions are not correct.";
                case 0x24: return "The request sequence is wrong.";
                case 0x31: return "The request is out of range.";
                case 0x33: return "Security access was denied.";
                case 0x35: return "The key was rejected.";
                case 0x36: return "Too many security access attempts.";
                case 0x37: return "The required delay has not elapsed.";
                case 0x78: return "The ECU response is still pending.";
                case 0x7E: return "The sub-function is not supported in this session.";
                case 0x7F: return "The service is not supported in the active session.";
                default: return "The ECU returned negative response " + code.ToString("X2") + ".";
            }
        }
    }
}
