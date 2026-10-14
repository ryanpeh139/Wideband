using System;
using System.Threading;

namespace Wideband
{
    // Runs a flash job off the UI thread and posts progress back to the captured context.
    sealed class FlashWorker
    {
        readonly object gate = new object();
        readonly SynchronizationContext context;
        Thread thread;
        volatile bool cancel;

        public FlashWorker()
        {
            context = SynchronizationContext.Current;
            if (context == null) context = new SynchronizationContext();
        }

        public bool IsRunning
        {
            get { return thread != null && thread.IsAlive; }
        }

        public void Start(LinkSession session, IDiagnosticLink link, FlashPlan plan, FlashProgressHandler progress, Action<FlashResult> completed)
        {
            lock (gate)
            {
                if (IsRunning) throw new InvalidOperationException("A flash job is already running.");
                cancel = false;
                var job = new FlashJob(link, delegate (FlashProgress update)
                {
                    FlashProgress copy = Copy(update);
                    context.Post(delegate
                    {
                        if (progress != null) progress(copy);
                    }, null);
                }, delegate { return !cancel; });
                thread = new Thread(delegate ()
                {
                    FlashResult result;
                    try
                    {
                        result = job.Run(plan, session);
                    }
                    catch (Exception ex)
                    {
                        result = new FlashResult();
                        result.Stage = FlashStage.Failed;
                        result.Error = ex.Message;
                    }
                    context.Post(delegate
                    {
                        if (completed != null) completed(result);
                    }, null);
                });
                thread.IsBackground = true;
                thread.Name = "Wideband flash";
                thread.Start();
            }
        }

        public void Cancel()
        {
            cancel = true;
        }

        static FlashProgress Copy(FlashProgress source)
        {
            var copy = new FlashProgress();
            copy.Stage = source.Stage;
            copy.Percent = source.Percent;
            copy.KilobytesPerSecond = source.KilobytesPerSecond;
            copy.BytesSent = source.BytesSent;
            copy.TotalBytes = source.TotalBytes;
            copy.Detail = source.Detail;
            return copy;
        }
    }
}
