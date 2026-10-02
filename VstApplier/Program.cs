namespace VstApplier
{
    internal static class Program
    {
        private const string SingleInstanceMutexName = "VstApplier.SingleInstance";
        private const string ShowWindowEventName = "VstApplier.ShowWindow";

        /// <summary>
        ///  The main entry point for the application.
        /// </summary>
        [STAThread]
        static void Main(string[] args)
        {
            using var singleInstance = new Mutex(initiallyOwned: true, SingleInstanceMutexName, out var isFirstInstance);

            if (!isFirstInstance)
            {
                SignalExistingInstance(args);
                return;
            }

            // To customize application configuration such as set high DPI settings or default font,
            // see https://aka.ms/applicationconfiguration.
            ApplicationConfiguration.Initialize();
            var mainForm = new MainForm();
            StartShowWindowListener(mainForm);
            Application.Run(mainForm);
        }

        /// <summary>
        /// A second launch (for example from the desktop shortcut) wakes the running
        /// instance instead of starting another copy. Autostart duplicates stay quiet.
        /// </summary>
        private static void SignalExistingInstance(string[] args)
        {
            var startHiddenInTray = args.Any(argument =>
                string.Equals(argument, "--tray", StringComparison.OrdinalIgnoreCase));
            if (startHiddenInTray)
            {
                return;
            }

            try
            {
                using var showEvent = EventWaitHandle.OpenExisting(ShowWindowEventName);
                showEvent.Set();
            }
            catch (WaitHandleCannotBeOpenedException)
            {
                // The other instance is still starting up; nothing to signal.
            }
        }

        private static void StartShowWindowListener(MainForm mainForm)
        {
            var listener = new Thread(() =>
            {
                using var showEvent = new EventWaitHandle(
                    initialState: false,
                    EventResetMode.AutoReset,
                    ShowWindowEventName);

                while (true)
                {
                    showEvent.WaitOne();

                    try
                    {
                        mainForm.BeginInvoke(mainForm.RestoreFromTray);
                    }
                    catch (ObjectDisposedException)
                    {
                        break;
                    }
                    catch (InvalidOperationException)
                    {
                        // The form handle is not ready yet; the next signal will retry.
                    }
                }
            })
            {
                IsBackground = true,
                Name = "ShowWindowListener",
            };

            listener.Start();
        }
    }
}
