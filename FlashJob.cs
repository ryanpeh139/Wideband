using System;

namespace Wideband
{
    // Caller-supplied flash sequence. This file does not calculate a key, pick a routine id, or suppress a trouble code.
    enum FlashStage
    {
        Idle,
        Detect,
        Security,
        Read,
        Erase,
        Transfer,
        Checksum,
        Reset,
        Complete,
        Failed,
        Aborted
    }

    sealed class FlashProgress
    {
        public FlashStage Stage;
        public int Percent;
        public double KilobytesPerSecond;
        public int BytesSent;
        public int TotalBytes;
        public string Detail = "";
    }

    delegate void FlashProgressHandler(FlashProgress progress);

    sealed class FlashPlan
    {
        public long Address = 0;
        public byte[] Image = null;
        public ISecurityKey Keys = null;
        public byte SecurityLevel = 0x01;
        public bool ProgrammingSession = true;
        public bool ReadBack = false;
        public bool Erase = false;
        public ushort RoutineId = 0;
        public bool ResetWhenComplete = false;
        public byte SessionType = 0;
        public DiagnosticProtocol Diagnostic = DiagnosticProtocol.Uds;
        public IChecksum Checksum = null;
        public int ChunkBytes = 256;
        public int AddressBytes = 4;
        public int SizeBytes = 4;
        public int MaxPending = 10;
    }

    sealed class FlashResult
    {
        public FlashStage Stage = FlashStage.Idle;
        public bool Complete;
        public bool Canceled;
        public string Error = "";
        public byte NegativeResponse;
        public int BytesSent;
        public byte[] ReadData = new byte[0];
        public bool ChannelClosed;
    }

    sealed class FlashJob
    {
        readonly IDiagnosticLink link;
        readonly FlashProgressHandler progress;
        readonly Func<bool> shouldContinue;
        int totalBytes;
        int transferStart;
        double kilobytesPerSecond;

        public FlashJob(IDiagnosticLink link, FlashProgressHandler progress)
            : this(link, progress, null)
        {
        }

        public FlashJob(IDiagnosticLink link, FlashProgressHandler progress, Func<bool> shouldContinue)
        {
            if (link == null) throw new ArgumentNullException("link");
            this.link = link;
            this.progress = progress;
            this.shouldContinue = shouldContinue;
        }

        public FlashResult Run(FlashPlan plan, LinkSession session)
        {
            Validate(plan);
            totalBytes = plan.Image.Length;
            var result = new FlashResult();
            bool bus = false;
            FlashStage stage = FlashStage.Detect;
            try
            {
                if (plan.Checksum != null && !plan.Checksum.Verify(plan.Image))
                {
                    Fail(result, FlashStage.Checksum, "The image checksum does not match.");
                    return result;
                }
                var client = new UdsClient(link, plan.MaxPending);
                bus = true;
                stage = FlashStage.Detect;
                Report(FlashStage.Detect, 5, 0, "Looking for an ECU.");
                DiagnosticService.TesterPresent(client, plan.Diagnostic);
                if (!Continue(result)) return result;

                stage = FlashStage.Security;
                Report(FlashStage.Security, 15, 0, "Requesting the diagnostic session.");
                byte sessionType = plan.SessionType;
                if (sessionType == 0) sessionType = plan.ProgrammingSession ? (byte)0x02 : (byte)0x03;
                var access = new ExtendedSessionAccess(link, plan.Keys, plan.SecurityLevel, plan.MaxPending, sessionType);
                DiagnosticAccessResult opened = access.Run();
                if (!opened.Unlocked)
                {
                    result.NegativeResponse = opened.NegativeResponse;
                    Fail(result, FlashStage.Security, opened.Error.Length == 0 ? "Security access failed." : opened.Error);
                    return result;
                }
                int timeout = opened.P2StarMs > 0 ? opened.P2StarMs : opened.P2Ms;
                if (timeout > 0) client.SetTimers(opened.P2Ms, timeout);
                if (!Continue(result)) return result;

                if (plan.ReadBack)
                {
                    stage = FlashStage.Read;
                    Report(FlashStage.Read, 25, 0, "Reading memory.");
                    result.ReadData = UdsServices.ReadRange(client, plan.Address, plan.Image.Length, plan.AddressBytes, plan.SizeBytes, plan.ChunkBytes, delegate (int sent, int total)
                    {
                        int percent = 25;
                        if (total > 0) percent = 25 + (int)(15L * sent / total);
                        Report(FlashStage.Read, percent, sent, "Reading memory.");
                    });
                    if (!Continue(result)) return result;
                }

                if (plan.Erase)
                {
                    stage = FlashStage.Erase;
                    Report(FlashStage.Erase, 45, 0, "Starting the erase routine.");
                    UdsServices.StartRoutine(client, plan.RoutineId, plan.Address, plan.Image.Length, plan.AddressBytes, plan.SizeBytes);
                    if (!Continue(result)) return result;
                }

                stage = FlashStage.Transfer;
                transferStart = Environment.TickCount;
                Report(FlashStage.Transfer, 55, 0, "Transferring the image.");
                var download = new FlashDownload(link, delegate (int sent, int total)
                {
                    NoteRate(sent);
                    int percent = 55;
                    if (total > 0) percent = 55 + (int)(35L * sent / total);
                    Report(FlashStage.Transfer, percent, sent, "Transferring the image.");
                }, plan.ChunkBytes, plan.AddressBytes, plan.SizeBytes, plan.MaxPending);
                download.ShouldContinue = shouldContinue;
                DownloadResult transferred = download.Run(plan.Address, plan.Image);
                result.BytesSent = transferred.BytesSent;
                if (!transferred.Complete)
                {
                    result.NegativeResponse = transferred.NegativeResponse;
                    if (transferred.Error.IndexOf("canceled") >= 0)
                    {
                        result.Canceled = true;
                        Fail(result, FlashStage.Aborted, transferred.Error);
                    }
                    else
                    {
                        Fail(result, FlashStage.Transfer, transferred.Error);
                    }
                    return result;
                }
                if (!Continue(result)) return result;

                if (plan.Checksum != null)
                {
                    stage = FlashStage.Checksum;
                    Report(FlashStage.Checksum, 94, result.BytesSent, "Checking the image.");
                    if (!plan.Checksum.Verify(plan.Image))
                    {
                        Fail(result, FlashStage.Checksum, "The image checksum does not match.");
                        return result;
                    }
                }

                if (plan.ResetWhenComplete)
                {
                    stage = FlashStage.Reset;
                    Report(FlashStage.Reset, 98, result.BytesSent, "Resetting the ECU.");
                    UdsServices.HardReset(client);
                }

                result.Complete = true;
                result.Stage = FlashStage.Complete;
                result.BytesSent = plan.Image.Length;
                Report(FlashStage.Complete, 100, result.BytesSent, "The transfer finished.");
                return result;
            }
            catch (UdsException ex)
            {
                result.NegativeResponse = ex.NegativeResponse;
                Fail(result, stage, ex.Message);
                return result;
            }
            catch (IsoTpException ex)
            {
                Fail(result, stage, ex.Message);
                return result;
            }
            catch (DownloadHaltException ex)
            {
                result.Canceled = true;
                Fail(result, FlashStage.Aborted, ex.Message);
                return result;
            }
            finally
            {
                if (bus && !result.Complete && session != null && session.State == LinkState.Connected)
                {
                    try
                    {
                        session.Recover();
                        result.ChannelClosed = true;
                    }
                    catch (Exception ex)
                    {
                        result.Error = result.Error + " Channel cleanup failed: " + ex.Message;
                    }
                }
            }
        }

        bool Continue(FlashResult result)
        {
            if (shouldContinue == null || shouldContinue()) return true;
            result.Canceled = true;
            Fail(result, FlashStage.Aborted, "The transfer was canceled.");
            return false;
        }

        void NoteRate(int sent)
        {
            int elapsed = unchecked(Environment.TickCount - transferStart);
            if (elapsed < 1) elapsed = 1;
            kilobytesPerSecond = (sent / 1024.0) / (elapsed / 1000.0);
        }

        void Report(FlashStage stage, int percent, int sent, string detail)
        {
            if (percent < 0) percent = 0;
            if (percent > 100) percent = 100;
            var update = new FlashProgress();
            update.Stage = stage;
            update.Percent = percent;
            update.KilobytesPerSecond = kilobytesPerSecond;
            update.BytesSent = sent;
            update.TotalBytes = totalBytes;
            update.Detail = detail;
            if (progress != null) progress(update);
        }

        void Fail(FlashResult result, FlashStage stage, string error)
        {
            result.Complete = false;
            result.Stage = stage;
            result.Error = error == null ? "" : error;
            Report(stage, 0, result.BytesSent, result.Error);
        }

        static void Validate(FlashPlan plan)
        {
            if (plan == null) throw new ArgumentNullException("plan");
            if (plan.Image == null || plan.Image.Length == 0)
                throw new ArgumentException("The download image is empty.");
            if (plan.Keys == null) throw new ArgumentNullException("plan.Keys");
            if (plan.ChunkBytes < 1 || plan.ChunkBytes > 4095)
                throw new ArgumentOutOfRangeException("plan", "A transfer block must hold 1 to 4095 image bytes.");
            if (plan.AddressBytes < 1 || plan.AddressBytes > 4 || plan.SizeBytes < 1 || plan.SizeBytes > 4)
                throw new ArgumentOutOfRangeException("plan", "Address and size fields must be 1 to 4 bytes.");
            if (!UdsEncoding.Fits(plan.Address, plan.AddressBytes) || !UdsEncoding.Fits(plan.Image.Length, plan.SizeBytes))
                throw new ArgumentOutOfRangeException("plan", "The address or image length does not fit in its field.");
            if (!UdsEncoding.Fits(plan.ChunkBytes, plan.SizeBytes))
                throw new ArgumentOutOfRangeException("plan", "The transfer chunk does not fit in the size field.");
            if (plan.Erase && plan.RoutineId == 0)
                throw new ArgumentOutOfRangeException("plan", "An erase routine id is required when erase is requested.");
            if (plan.MaxPending < 1)
                throw new ArgumentOutOfRangeException("plan", "The pending-response limit must be at least 1.");
        }
    }
}
