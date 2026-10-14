using System;

namespace Wideband
{
    // ISO 14229 download sequencing: 0x34, 0x36, 0x37.
    // The image bytes and address come from the caller. This file does not erase memory or calculate a checksum.
    enum DownloadStep
    {
        Idle,
        RequestDownload,
        WaitDownload,
        TransferBlock,
        WaitTransfer,
        RequestExit,
        WaitExit,
        Complete,
        Failed
    }

    delegate void DownloadProgressHandler(int bytesSent, int totalBytes);

    sealed class DownloadResult
    {
        public DownloadStep Step;
        public DownloadStep FailedDuring;
        public bool Complete;
        public int BytesSent;
        public int TotalBytes;
        public int BlocksSent;
        public byte NegativeResponse;
        public string Error = "";
    }

    sealed class FlashDownload
    {
        const byte Negative = 0x7F;
        const byte ResponsePending = 0x78;
        readonly IDiagnosticLink link;
        readonly DownloadProgressHandler progress;
        readonly int chunkBytes;
        readonly int addressBytes;
        readonly int sizeBytes;
        readonly int maxPending;
        DownloadStep step = DownloadStep.Idle;
        DownloadStep failedDuring = DownloadStep.Idle;
        byte[] image;
        byte[] downloadRequest;
        int totalBytes;
        int bytesSent;
        int offset;
        int blockBudget;
        int pendingChunk;
        int sequence;
        int sentSequence;
        int blocksSent;
        int p2Ms;
        int p2StarMs;
        byte negative;
        string error = "";

        public FlashDownload(IDiagnosticLink link, DownloadProgressHandler progress)
            : this(link, progress, 256, 4, 4, 10)
        {
        }

        public FlashDownload(IDiagnosticLink link, DownloadProgressHandler progress, int chunkBytes, int addressBytes, int sizeBytes, int maxPending)
        {
            if (link == null) throw new ArgumentNullException("link");
            if (chunkBytes < 1 || chunkBytes > 4095)
                throw new ArgumentOutOfRangeException("chunkBytes", "A transfer block must hold 1 to 4095 image bytes.");
            if (addressBytes < 1 || addressBytes > 4)
                throw new ArgumentOutOfRangeException("addressBytes", "The address length must be 1 to 4 bytes.");
            if (sizeBytes < 1 || sizeBytes > 4)
                throw new ArgumentOutOfRangeException("sizeBytes", "The size length must be 1 to 4 bytes.");
            if (maxPending < 1)
                throw new ArgumentOutOfRangeException("maxPending", "The pending-response limit must be at least 1.");
            this.link = link;
            this.progress = progress;
            this.chunkBytes = chunkBytes;
            this.addressBytes = addressBytes;
            this.sizeBytes = sizeBytes;
            this.maxPending = maxPending;
        }

        public Func<bool> ShouldContinue;

        public DownloadStep Step
        {
            get { return step; }
        }

        bool Allowed()
        {
            if (ShouldContinue == null || ShouldContinue()) return true;
            Fail("The download was canceled.");
            return false;
        }

        public DownloadResult Run(long address, byte[] image)
        {
            if (image == null) throw new ArgumentNullException("image");
            if (image.Length == 0) throw new ArgumentException("The download image is empty.");
            if (!Fits(address, addressBytes))
                throw new ArgumentOutOfRangeException("address", "The address does not fit in the address field.");
            if (!Fits(image.Length, sizeBytes))
                throw new ArgumentOutOfRangeException("image", "The image length does not fit in the size field.");
            if (step != DownloadStep.Idle && step != DownloadStep.Failed && step != DownloadStep.Complete)
                throw new InvalidOperationException("A download is already running.");
            this.image = image;
            totalBytes = image.Length;
            bytesSent = 0;
            offset = 0;
            blocksSent = 0;
            sequence = 1;
            sentSequence = 0;
            pendingChunk = 0;
            blockBudget = 0;
            negative = 0;
            error = "";
            failedDuring = DownloadStep.Idle;
            p2Ms = link.TimeoutMs;
            p2StarMs = link.TimeoutMs;
            downloadRequest = BuildDownload(address, image.Length);
            step = DownloadStep.RequestDownload;
            while (step != DownloadStep.Complete && step != DownloadStep.Failed)
                Advance();
            return Snapshot();
        }

        void Advance()
        {
            switch (step)
            {
                case DownloadStep.RequestDownload:
                    if (!Allowed()) break;
                    if (Send(downloadRequest))
                        step = DownloadStep.WaitDownload;
                    break;
                case DownloadStep.WaitDownload:
                    OnDownload(Receive());
                    break;
                case DownloadStep.TransferBlock:
                    if (offset >= totalBytes)
                    {
                        step = DownloadStep.RequestExit;
                        break;
                    }
                    if (!Allowed()) break;
                    pendingChunk = blockBudget;
                    if (pendingChunk > totalBytes - offset) pendingChunk = totalBytes - offset;
                    if (Send(BuildTransfer(offset, pendingChunk)))
                        step = DownloadStep.WaitTransfer;
                    break;
                case DownloadStep.WaitTransfer:
                    OnTransfer(Receive());
                    break;
                case DownloadStep.RequestExit:
                    if (!Allowed()) break;
                    if (Send(new byte[] { 0x37 }))
                        step = DownloadStep.WaitExit;
                    break;
                case DownloadStep.WaitExit:
                    OnExit(Receive());
                    break;
                default:
                    Fail("A download is not running.");
                    break;
            }
        }

        void OnDownload(byte[] response)
        {
            if (step == DownloadStep.Failed) return;
            if (response == null)
            {
                Fail("Timed out waiting for the download response.");
                return;
            }
            if (response.Length >= 3 && response[0] == Negative)
            {
                FailNegative(response, 0x34);
                return;
            }
            int maxBlock;
            if (!TryMaxBlock(response, out maxBlock))
            {
                Fail("The download response was not valid.");
                return;
            }
            if (maxBlock < 3)
            {
                Fail("The ECU block length is too small.");
                return;
            }
            int budget = maxBlock - 2;
            if (budget > chunkBytes) budget = chunkBytes;
            blockBudget = budget;
            offset = 0;
            sequence = 1;
            Report(0);
            if (step == DownloadStep.Failed) return;
            step = DownloadStep.TransferBlock;
        }

        void OnTransfer(byte[] response)
        {
            if (step == DownloadStep.Failed) return;
            if (response == null)
            {
                Fail("Timed out waiting for a transfer response.");
                return;
            }
            if (response.Length >= 3 && response[0] == Negative)
            {
                FailNegative(response, 0x36);
                return;
            }
            if (response.Length < 2 || response[0] != 0x76 || response[1] != (byte)sentSequence)
            {
                Fail("The block sequence counter did not match.");
                return;
            }
            offset += pendingChunk;
            bytesSent = offset;
            blocksSent++;
            Report(bytesSent);
            if (step == DownloadStep.Failed) return;
            sequence = (sequence + 1) & 0xFF;
            step = DownloadStep.TransferBlock;
        }

        void OnExit(byte[] response)
        {
            if (step == DownloadStep.Failed) return;
            if (response == null)
            {
                Fail("Timed out waiting for the transfer exit.");
                return;
            }
            if (response.Length >= 3 && response[0] == Negative)
            {
                FailNegative(response, 0x37);
                return;
            }
            if (response.Length < 1 || response[0] != 0x77)
            {
                Fail("The transfer exit response was not valid.");
                return;
            }
            Report(bytesSent);
            if (step == DownloadStep.Failed) return;
            step = DownloadStep.Complete;
        }

        byte[] BuildDownload(long address, int size)
        {
            byte[] message = new byte[3 + addressBytes + sizeBytes];
            message[0] = 0x34;
            message[1] = 0x00;
            message[2] = (byte)((sizeBytes << 4) | addressBytes);
            WriteBigEndian(message, 3, addressBytes, address);
            WriteBigEndian(message, 3 + addressBytes, sizeBytes, size);
            return message;
        }

        byte[] BuildTransfer(int start, int count)
        {
            byte[] message = new byte[2 + count];
            message[0] = 0x36;
            message[1] = (byte)sequence;
            sentSequence = sequence;
            Buffer.BlockCopy(image, start, message, 2, count);
            return message;
        }

        // maxNumberOfBlockLength includes the service id and the sequence counter.
        bool TryMaxBlock(byte[] response, out int maxBlock)
        {
            maxBlock = 0;
            if (response == null || response.Length < 3 || response[0] != 0x74) return false;
            int count = (response[1] >> 4) & 0x0F;
            if (count < 1 || count > 4) return false;
            if (response.Length < 2 + count) return false;
            long value = 0;
            for (int i = 0; i < count; i++)
                value = (value << 8) | response[2 + i];
            if (value < 0) return false;
            if (value > chunkBytes + 2L) value = chunkBytes + 2L;
            maxBlock = (int)value;
            return true;
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

        void Report(int sent)
        {
            if (progress == null) return;
            try
            {
                progress(sent, totalBytes);
            }
            catch (DownloadHaltException ex)
            {
                Fail(ex.Message);
            }
            catch (Exception)
            {
                Fail("The progress callback failed.");
            }
        }

        void FailNegative(byte[] response, byte expectedService)
        {
            if (response[1] != expectedService)
            {
                Fail("The ECU rejected a different service.");
                return;
            }
            negative = response[2];
            Fail(NegativeText(negative));
        }

        void Fail(string message)
        {
            if (step == DownloadStep.Failed) return;
            failedDuring = step;
            error = message;
            step = DownloadStep.Failed;
        }

        DownloadResult Snapshot()
        {
            var result = new DownloadResult();
            result.Step = step;
            result.FailedDuring = failedDuring;
            result.Complete = step == DownloadStep.Complete;
            result.BytesSent = bytesSent;
            result.TotalBytes = totalBytes;
            result.BlocksSent = blocksSent;
            result.NegativeResponse = negative;
            result.Error = error == null ? "" : error;
            return result;
        }

        static void WriteBigEndian(byte[] target, int offset, int length, long value)
        {
            for (int i = length - 1; i >= 0; i--)
            {
                target[offset + i] = (byte)(value & 0xFF);
                value >>= 8;
            }
        }

        static bool Fits(long value, int length)
        {
            if (value < 0 || length < 1 || length > 4) return false;
            long max = (1L << (length * 8)) - 1;
            return value <= max;
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
                case 0x70: return "The download was not accepted.";
                case 0x71: return "The transfer was suspended.";
                case 0x72: return "The transfer failed.";
                case 0x73: return "The block sequence counter was rejected.";
                case 0x78: return "The ECU response is still pending.";
                case 0x7F: return "The service is not supported in the active session.";
                default: return "The ECU returned negative response " + code.ToString("X2") + ".";
            }
        }
    }

    sealed class DownloadHaltException : Exception
    {
        public DownloadHaltException(string message)
            : base(message)
        {
        }
    }
}
