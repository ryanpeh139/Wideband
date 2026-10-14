using System;
using System.Threading;

namespace Wideband
{
    // One open cable. Device and channel ids stay valid until Disconnect.
    // TesterPresent runs every two seconds unless a transfer owns the channel.
    // The security key comes from ComputeKey or security-config.json. This file does not embed a mask.
    sealed class ConfiguredKey : ISecurityKey
    {
        readonly ComputeSecurityKey compute;

        public ConfiguredKey(ComputeSecurityKey compute)
        {
            this.compute = compute;
        }

        public byte[] Calculate(byte[] seed)
        {
            if (compute == null)
                throw new InvalidOperationException("No security key function is configured.");
            return compute(seed);
        }
    }

    sealed class VehicleLink
    {
        public static readonly VehicleLink Current = new VehicleLink();
        readonly object gate = new object();
        ICanLink channel;
        IDiagnosticLink diagnostic;
        EcuProfile profile;
        Timer timer;
        int busy;
        bool connected;
        bool cancel;
        uint deviceId;
        uint channelId;
        string firmware = "";
        string lastNote = "";
        ComputeKey computeKey;

        public event Action<string> Noted;
        public event Action<string> Traced;

        public bool IsConnected
        {
            get { return connected; }
        }

        public uint DeviceId
        {
            get { return deviceId; }
        }

        public uint ChannelId
        {
            get { return channelId; }
        }

        public string Firmware
        {
            get { return firmware == null ? "" : firmware; }
        }

        public EcuProfile Profile
        {
            get { return profile; }
        }

        public DiagnosticProtocol Protocol
        {
            get { return profile == null ? DiagnosticProtocol.Uds : profile.Diagnostic; }
            set
            {
                if (profile == null) return;
                profile.Diagnostic = value;
            }
        }

        public void SetSecurityKey(ComputeKey compute)
        {
            computeKey = compute;
        }

        public void ConnectAndHold(CanChannelSettings settings, EcuProfile selected)
        {
            if (settings == null) throw new ArgumentNullException("settings");
            if (selected == null) throw new ArgumentNullException("selected");
            Disconnect();
            var openSettings = new CanChannelSettings();
            openSettings.J2534LibraryPath = settings.J2534LibraryPath;
            openSettings.Bitrate = selected.Bitrate;
            openSettings.ProtocolId = selected.ProtocolId;
            openSettings.ConnectFlags = selected.ConnectFlags;
            openSettings.ExtendedId = selected.ExtendedId;
            if (selected.Bus == VehicleBus.CanIsoTp)
            {
                openSettings.FilterId = selected.ResponseId;
                openSettings.FilterMask = selected.FilterMask;
            }
            var link = new J2534CanLink();
            try
            {
                link.Open(openSettings);
                IDiagnosticLink pipe = OpenPipe(link, selected);
                lock (gate)
                {
                    channel = link;
                    diagnostic = new TracingLink(pipe, Trace);
                    profile = selected;
                    deviceId = link.DeviceId;
                    channelId = link.ChannelId;
                    firmware = link.Firmware == null ? "" : link.Firmware;
                    connected = true;
                    cancel = false;
                }
                StartTimer(2000);
                Note(HoldText());
            }
            catch
            {
                try { link.Abort(); }
                catch (Exception) { }
                try { link.Dispose(); }
                catch (Exception) { }
                lock (gate)
                {
                    if (channel == link) channel = null;
                    connected = false;
                    deviceId = 0;
                    channelId = 0;
                    diagnostic = null;
                }
                throw;
            }
        }

        public void Hold(IDiagnosticLink link, EcuProfile selected, int intervalMs)
        {
            if (link == null) throw new ArgumentNullException("link");
            if (selected == null) throw new ArgumentNullException("selected");
            Disconnect();
            lock (gate)
            {
                diagnostic = new TracingLink(link, Trace);
                profile = selected;
                connected = true;
                cancel = false;
                deviceId = 0;
                channelId = 0;
            }
            StartTimer(intervalMs);
        }

        public void PauseKeepAlive()
        {
            Interlocked.Exchange(ref busy, 1);
        }

        public void ResumeKeepAlive()
        {
            Interlocked.Exchange(ref busy, 0);
        }

        public byte[] ReadCalibration(long address, int length, int chunkBytes, FlashProgressHandler progress)
        {
            IDiagnosticLink link;
            EcuProfile selected = BeginWork(address, length, chunkBytes, false, out link);
            try
            {
                var client = new UdsClient(link, 10);
                Report(progress, FlashStage.Detect, 5, 0, length, "Looking for an ECU.");
                DiagnosticService.TesterPresent(client, selected.Diagnostic);
                if (cancel) throw new InvalidOperationException("The read was canceled.");
                Report(progress, FlashStage.Security, 15, 0, length, "Requesting the diagnostic session.");
                var access = new ExtendedSessionAccess(link, KeysFor(selected), 0x01, 10, selected.ReadSession);
                DiagnosticAccessResult opened = access.Run();
                if (!opened.Unlocked)
                    throw new InvalidOperationException(opened.Error.Length == 0 ? "Security access failed." : opened.Error);
                int timeout = opened.P2StarMs > 0 ? opened.P2StarMs : opened.P2Ms;
                if (timeout > 0) client.SetTimers(opened.P2Ms, timeout);
                if (cancel) throw new InvalidOperationException("The read was canceled.");
                Report(progress, FlashStage.Read, 25, 0, length, "Reading memory.");
                return UdsServices.ReadRange(client, address, length, selected.AddressBytes, selected.SizeBytes, chunkBytes, delegate (int sent, int total)
                {
                    int percent = 25;
                    if (total > 0) percent = 25 + (int)(70L * sent / total);
                    Report(progress, FlashStage.Read, percent, sent, total, "Reading memory.");
                });
            }
            finally
            {
                cancel = false;
                Interlocked.Exchange(ref busy, 0);
            }
        }

        public FlashResult Transfer(long address, byte[] image, int chunkBytes, FlashProgressHandler progress)
        {
            if (image == null || image.Length == 0) throw new ArgumentException("The download image is empty.");
            IDiagnosticLink link;
            EcuProfile selected = BeginWork(address, image.Length, chunkBytes, true, out link);
            try
            {
                PreparedImage prepared = PlatformSecurityAndChecksum.PrepareForFlash(selected.Id, selected.Name, image, address);
                var plan = new FlashPlan();
                plan.Address = address;
                plan.Image = prepared.Bytes;
                plan.Keys = KeysFor(selected);
                plan.Checksum = prepared.Checksum;
                plan.ProgrammingSession = true;
                plan.SessionType = selected.ProgrammingSession;
                plan.Diagnostic = selected.Diagnostic;
                plan.ChunkBytes = chunkBytes;
                plan.AddressBytes = selected.AddressBytes;
                plan.SizeBytes = selected.SizeBytes;
                plan.Erase = false;
                plan.ResetWhenComplete = true;
                var job = new FlashJob(link, progress, delegate { return !cancel; });
                return job.Run(plan, null);
            }
            finally
            {
                cancel = false;
                Interlocked.Exchange(ref busy, 0);
            }
        }

        public void Disconnect()
        {
            cancel = true;
            connected = false;
            StopTimer();
            ICanLink open;
            lock (gate)
            {
                open = channel;
                channel = null;
                diagnostic = null;
                profile = null;
                deviceId = 0;
                channelId = 0;
                firmware = "";
            }
            var j2534 = open as J2534CanLink;
            if (j2534 != null) j2534.Abort();
            if (open != null)
            {
                try { open.Dispose(); }
                catch (Exception) { }
            }
            Note("Disconnected.");
        }

        static IDiagnosticLink OpenPipe(J2534CanLink link, EcuProfile selected)
        {
            if (selected.Bus == VehicleBus.KLine)
                return new KLineLink(link, selected.Ecu, selected.Tester, 1000);
            if (selected.Bus == VehicleBus.Vpw)
                return new VpwLink(link, selected.VpwHeader, selected.Ecu, selected.Tester, 1000);
            var config = new IsoTpConfig();
            config.TransmitId = selected.RequestId;
            config.ReceiveId = selected.ResponseId;
            config.ExtendedId = selected.ExtendedId;
            config.TimeoutMs = 1000;
            return new IsoTpLink(link, config);
        }

        EcuProfile BeginWork(long address, int length, int chunkBytes, bool limitWholeLength, out IDiagnosticLink link)
        {
            if (!connected) throw new InvalidOperationException("The channel is not connected.");
            if (length < 1) throw new ArgumentOutOfRangeException("length", "The transfer length must be at least one byte.");
            EcuProfile selected;
            lock (gate)
            {
                link = diagnostic;
                selected = profile;
            }
            if (link == null || selected == null) throw new InvalidOperationException("The channel is not connected.");
            if (chunkBytes < 1 || chunkBytes > selected.MaxChunk)
                throw new ArgumentOutOfRangeException("chunkBytes", "This profile transfers at most " + selected.MaxChunk.ToString() + " image bytes in one block.");
            if (!UdsEncoding.Fits(chunkBytes, selected.SizeBytes))
                throw new ArgumentOutOfRangeException("chunkBytes", "The chunk does not fit in this profile's size field.");
            if (!UdsEncoding.Fits(address, selected.AddressBytes) || !UdsEncoding.Fits(address + length - 1, selected.AddressBytes))
                throw new ArgumentOutOfRangeException("address", "The address does not fit in this profile's address field.");
            if (limitWholeLength)
            {
                long maxSize = (1L << (selected.SizeBytes * 8)) - 1L;
                if (length > maxSize)
                    throw new ArgumentOutOfRangeException("length", "This profile can name at most " + maxSize.ToString() + " bytes in one download.");
            }
            if (Interlocked.CompareExchange(ref busy, 1, 0) != 0)
                throw new InvalidOperationException("The channel is busy.");
            return selected;
        }

        ISecurityKey KeysFor(EcuProfile selected)
        {
            ComputeKey local = computeKey;
            EcuProfile current = selected;
            return new ConfiguredKey(delegate (byte[] seed)
            {
                return PlatformSecurityAndChecksum.Reveal(seed, current, local, Trace);
            });
        }

        static void Report(FlashProgressHandler progress, FlashStage stage, int percent, int sent, int total, string detail)
        {
            if (progress == null) return;
            var update = new FlashProgress();
            update.Stage = stage;
            update.Percent = percent;
            update.BytesSent = sent;
            update.TotalBytes = total;
            update.Detail = detail;
            progress(update);
        }

        string HoldText()
        {
            string name = profile == null ? "the cable" : profile.Name;
            string text = "Session held on " + name + ". Device " + deviceId.ToString() + ", channel " + channelId.ToString() + ".";
            if (firmware != null && firmware.Length > 0) text += " Firmware " + firmware + ".";
            return text;
        }

        void StartTimer(int intervalMs)
        {
            StopTimer();
            if (intervalMs < 1) intervalMs = 2000;
            timer = new Timer(KeepAliveTick, null, intervalMs, intervalMs);
        }

        void StopTimer()
        {
            Timer current = timer;
            timer = null;
            if (current != null)
            {
                try { current.Dispose(); }
                catch (Exception) { }
            }
        }

        void KeepAliveTick(object state)
        {
            if (!connected) return;
            if (Interlocked.CompareExchange(ref busy, 1, 0) != 0) return;
            IDiagnosticLink link;
            EcuProfile selected;
            lock (gate)
            {
                link = diagnostic;
                selected = profile;
            }
            if (link == null || selected == null || !connected)
            {
                Interlocked.Exchange(ref busy, 0);
                return;
            }
            try
            {
                DiagnosticService.TesterPresent(new UdsClient(link, 2), selected.Diagnostic);
                if (connected) Note(HoldText());
            }
            catch (Exception ex)
            {
                if (connected)
                    Note("Channel remains open. Tester present was not answered. " + ex.Message);
            }
            finally
            {
                Interlocked.Exchange(ref busy, 0);
            }
        }

        void Note(string text)
        {
            if (text == lastNote) return;
            lastNote = text;
            Action<string> handler = Noted;
            if (handler == null) return;
            try { handler(text); }
            catch (Exception) { }
        }

        void Trace(string text)
        {
            Action<string> handler = Traced;
            if (handler == null) return;
            try { handler(text); }
            catch (Exception) { }
        }
    }
}
