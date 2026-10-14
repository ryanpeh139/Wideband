using System;

namespace Wideband
{
    // Owns one CAN channel. Open failure and Recover both release the device.
    enum LinkState
    {
        Disconnected,
        Initializing,
        Connected,
        BusError
    }

    sealed class LinkSession : IDisposable
    {
        ICanLink channel;
        LinkState state = LinkState.Disconnected;

        public LinkState State
        {
            get { return state; }
        }

        public string Error = "";

        public ICanLink Channel
        {
            get
            {
                if (state != LinkState.Connected || channel == null)
                    throw new InvalidOperationException("The channel is not connected.");
                return channel;
            }
        }

        public void Connect(string backend, CanChannelSettings settings)
        {
            Check(settings);
            if (state == LinkState.Initializing)
                throw new InvalidOperationException("The channel is already initializing.");
            CloseChannel();
            state = LinkState.Initializing;
            Error = "";
            ICanLink created = null;
            try
            {
                created = CanLinks.Create(backend);
                created.Open(settings);
                channel = created;
                state = LinkState.Connected;
            }
            catch (Exception ex)
            {
                if (created != null)
                {
                    try { created.Dispose(); }
                    catch (Exception) { }
                }
                channel = null;
                Error = ex.Message;
                state = LinkState.BusError;
                throw;
            }
        }

        public void Connect(ICanLink created, CanChannelSettings settings)
        {
            if (created == null) throw new ArgumentNullException("created");
            Check(settings);
            if (state == LinkState.Initializing)
                throw new InvalidOperationException("The channel is already initializing.");
            CloseChannel();
            state = LinkState.Initializing;
            Error = "";
            try
            {
                created.Open(settings);
                channel = created;
                state = LinkState.Connected;
            }
            catch (Exception ex)
            {
                try { created.Dispose(); }
                catch (Exception) { }
                channel = null;
                Error = ex.Message;
                state = LinkState.BusError;
                throw;
            }
        }

        public void Recover()
        {
            CloseChannel();
            Error = "";
            state = LinkState.Disconnected;
        }

        public void Disconnect()
        {
            Recover();
        }

        public void Dispose()
        {
            Recover();
        }

        void CloseChannel()
        {
            ICanLink open = channel;
            channel = null;
            if (open == null) return;
            try { open.Dispose(); }
            catch (Exception) { }
        }

        static void Check(CanChannelSettings settings)
        {
            if (settings == null) throw new ArgumentNullException("settings");
            if (settings.Bitrate < 1 || settings.Bitrate > 1000000)
                throw new ArgumentOutOfRangeException("settings", "Bitrate must be between 1 and 1000000.");
        }
    }
}
