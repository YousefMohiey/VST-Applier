using System.Drawing.Drawing2D;
using System.Runtime.InteropServices;

namespace VstApplier
{
    public partial class MainForm : Form
    {
        // Palette: deep #05202B, mint #00FFC4, teal #00B4B5, pink #FF4D7B.
        private static readonly Color DarkAppBackground = Color.FromArgb(5, 32, 43);
        private static readonly Color DarkSidebarBackground = Color.FromArgb(7, 38, 49);
        private static readonly Color DarkSurface = Color.FromArgb(10, 47, 60);
        private static readonly Color DarkControlBackground = Color.FromArgb(4, 24, 33);
        private static readonly Color DarkButtonBackground = Color.FromArgb(14, 57, 71);
        private static readonly Color DarkButtonHover = Color.FromArgb(21, 76, 92);
        private static readonly Color DarkButtonPressed = Color.FromArgb(27, 92, 110);
        private static readonly Color DarkButtonDisabled = Color.FromArgb(9, 40, 51);
        private static readonly Color DarkButtonDisabledText = Color.FromArgb(77, 112, 120);
        private static readonly Color DarkBorder = Color.FromArgb(21, 76, 90);
        private static readonly Color DarkSoftBorder = Color.FromArgb(33, 104, 120);
        private static readonly Color DarkPrimaryText = Color.FromArgb(231, 246, 244);
        private static readonly Color DarkSecondaryText = Color.FromArgb(128, 176, 182);
        private static readonly Color DarkAccentMint = Color.FromArgb(0, 255, 196);
        private static readonly Color DarkAccentTeal = Color.FromArgb(0, 180, 181);
        private static readonly Color DarkDanger = Color.FromArgb(255, 77, 123);
        private static readonly Color DarkOnAccent = Color.FromArgb(5, 32, 43);
        private const int DarkCornerRadius = 5;
        private static readonly Size FixedMainClientSize = new(834, 508);

        private readonly AudioInputDeviceScanner _audioInputDeviceScanner = new();
        private readonly AudioOutputDeviceScanner _audioOutputDeviceScanner = new();
        private readonly VirtualCableService _virtualCableService = new();
        private readonly AudioRoutingService _audioRoutingService = new();
        private readonly VstPluginScanner _vstPluginScanner = new();
        private readonly System.Windows.Forms.Timer _levelTimer = new();
        private AudioInputLevelMonitor? _inputLevelMonitor;
        private readonly Dictionary<VstPluginChainItem, Form> _pluginEditorForms = new();
        private string _nativeVstStatus = "Native VST host unchecked";
        private string _lastAudioProcessingStatus = string.Empty;
        private bool _isRefreshingDevices;
        private bool _isUpdatingPluginChainChecks;
        private bool _isPluginChainCheckboxClick;
        private bool _isRefreshingProfiles;
        private string? _activeProfileName;
        private AppSettings _appSettings = new();
        private CableSessionGuard? _cableSessionGuard;
        private NotifyIcon? _trayIcon;
        private ContextMenuStrip? _trayMenu;
        private bool _isExiting;
        private bool _trayBalloonShown;
        private bool _startHiddenInTray;

        public MainForm()
        {
            InitializeComponent();
            ApplyFixedMainLayout();
            ApplyDarkTheme();
            var applicationIcon = LoadApplicationIcon();
            if (applicationIcon is not null)
            {
                Icon = applicationIcon;
            }

            inputDeviceComboBox.SelectedIndexChanged += InputDeviceComboBox_SelectedIndexChanged;
            outputDeviceComboBox.SelectedIndexChanged += OutputDeviceComboBox_SelectedIndexChanged;
            InitializeBufferSizeComboBox();
            _levelTimer.Interval = 33;
            _levelTimer.Tick += LevelTimer_Tick;
            _levelTimer.Start();

            _appSettings = AppSettingsStore.Load();
            _cableSessionGuard = new CableSessionGuard(
                () => outputDeviceComboBox.SelectedItem as AudioOutputDevice,
                _appSettings.KeepCableClean);
            _startHiddenInTray = Environment.GetCommandLineArgs()
                .Any(argument => string.Equals(argument, "--tray", StringComparison.OrdinalIgnoreCase));

            if (_appSettings.StartWithWindows)
            {
                try
                {
                    AutoStartManager.SetEnabled(true);
                }
                catch
                {
                    // A stale autostart entry is not fatal; the settings dialog can fix it.
                }
            }

            InitializeTrayIcon();

            var startupProfile = VoiceSetupStore.LoadSession();
            pluginFolderTextBox.Text = startupProfile?.PluginFolder is { Length: > 0 } savedPluginFolder &&
                Directory.Exists(savedPluginFolder)
                ? savedPluginFolder
                : GetDefaultPluginFolder();
            UpdateNativeVstStatus();
            RefreshPluginList();
            RefreshAudioDevices();
            ApplySessionPreferences(startupProfile);
            var (restoredPluginCount, restoreFailures) = RestorePluginChainFromProfile(startupProfile);
            if (restoredPluginCount > 0)
            {
                pluginStatusLabel.Text = restoreFailures.Count > 0
                    ? $"Session restored: {restoredPluginCount} plugin(s). Skipped: {string.Join(", ", restoreFailures)}"
                    : $"Session restored: {restoredPluginCount} plugin(s)";
            }

            RefreshProfileList(startupProfile?.ActiveProfile);
            _activeProfileName = startupProfile?.ActiveProfile;
        }

        [DllImport("dwmapi.dll")]
        private static extern int DwmSetWindowAttribute(IntPtr hwnd, int dwAttribute, ref int pvAttribute, int cbAttribute);

        protected override void OnHandleCreated(EventArgs e)
        {
            base.OnHandleCreated(e);
            EnableDarkTitleBar();
        }

        protected override void OnShown(EventArgs e)
        {
            base.OnShown(e);
            ApplyFixedMainLayout();

            if (_startHiddenInTray)
            {
                HideToTray();
            }

            if (_appSettings.AutoStartMicrophone)
            {
                TryStartAudioRouteFromCurrentSelection();
            }
        }

        private static Icon? LoadApplicationIcon()
        {
            try
            {
                var iconPath = Path.Combine(AppContext.BaseDirectory, "Assets", "app.ico");
                if (File.Exists(iconPath))
                {
                    return new Icon(iconPath);
                }

                return Icon.ExtractAssociatedIcon(Application.ExecutablePath);
            }
            catch
            {
                return null;
            }
        }

        private void EnableDarkTitleBar()
        {
            try
            {
                var enabled = 1;
                var result = DwmSetWindowAttribute(Handle, 20, ref enabled, sizeof(int));
                if (result != 0)
                {
                    DwmSetWindowAttribute(Handle, 19, ref enabled, sizeof(int));
                }
            }
            catch
            {
            }
        }

        private void ApplyFixedMainLayout()
        {
            SuspendLayout();
            leftPanel.SuspendLayout();
            mainPanel.SuspendLayout();
            foundPluginsGroupBox.SuspendLayout();
            pluginChainGroupBox.SuspendLayout();

            try
            {
                AutoScaleMode = AutoScaleMode.None;
                FormBorderStyle = FormBorderStyle.FixedSingle;
                MaximizeBox = false;

                MinimumSize = Size.Empty;
                MaximumSize = Size.Empty;
                ClientSize = FixedMainClientSize;

                leftPanel.Dock = DockStyle.None;
                leftPanel.Anchor = AnchorStyles.None;
                leftPanel.Bounds = new Rectangle(0, 0, 320, 508);

                mainPanel.Dock = DockStyle.None;
                mainPanel.Anchor = AnchorStyles.None;
                mainPanel.Bounds = new Rectangle(320, 0, 514, 508);

                SetFixedBounds(profileLabel, 20, 30, 100, 15);
                SetFixedBounds(profileComboBox, 20, 50, 250, 28);
                SetFixedBounds(createProfileButton, 278, 48, 66, 28);
                SetFixedBounds(saveProfileButton, 348, 48, 60, 28);
                SetFixedBounds(deleteProfileButton, 412, 48, 66, 28);
                SetFixedBounds(pluginFolderLabel, 20, 88, 100, 15);
                SetFixedBounds(pluginFolderTextBox, 20, 108, 300, 23);
                SetFixedBounds(browsePluginFolderButton, 325, 106, 82, 28);
                SetFixedBounds(scanPluginsButton, 412, 106, 82, 28);
                SetFixedBounds(foundPluginsGroupBox, 20, 138, 482, 124);
                SetFixedBounds(foundPluginsListBox, 14, 23, 454, 84);
                SetFixedBounds(pluginStatusLabel, 20, 271, 353, 19);
                SetFixedBounds(addPluginButton, 378, 269, 116, 23);
                SetFixedBounds(pluginChainGroupBox, 20, 300, 482, 168);
                SetFixedBounds(pluginChainListBox, 14, 23, 384, 110);
                SetFixedBounds(movePluginUpButton, 404, 23, 63, 23);
                SetFixedBounds(movePluginDownButton, 404, 51, 63, 23);
                SetFixedBounds(removePluginButton, 404, 79, 63, 23);
                SetFixedBounds(openPluginEditorButton, 404, 106, 63, 23);
                SetFixedBounds(settingsButton, 20, 471, 92, 26);
                SetFixedBounds(copyrightLabel, 152, 473, 342, 25);

                MinimumSize = Size;
                MaximumSize = Size;
            }
            finally
            {
                pluginChainGroupBox.ResumeLayout(false);
                foundPluginsGroupBox.ResumeLayout(false);
                mainPanel.ResumeLayout(false);
                leftPanel.ResumeLayout(false);
                ResumeLayout(false);
            }
        }

        private static void SetFixedBounds(Control control, int x, int y, int width, int height)
        {
            control.Anchor = AnchorStyles.None;
            control.Bounds = new Rectangle(x, y, width, height);
        }

        private void ApplyDarkTheme()
        {
            BackColor = DarkAppBackground;
            ForeColor = DarkPrimaryText;
            ApplyDarkThemeRecursive(this);

            leftPanel.BackColor = DarkSidebarBackground;
            mainPanel.BackColor = DarkAppBackground;
            startButton.Tag = "primary";
            stopButton.Tag = "danger";
            deleteProfileButton.Tag = "danger-outline";

            inputLevelMeter.BackColor = DarkControlBackground;
            outputLevelMeter.BackColor = DarkControlBackground;

            inputLevelStatusLabel.ForeColor = DarkSecondaryText;
            outputLevelStatusLabel.ForeColor = DarkSecondaryText;
            cableOutputValueLabel.ForeColor = DarkPrimaryText;
            cableInputValueLabel.ForeColor = DarkPrimaryText;
            cableStateValueLabel.ForeColor = DarkSecondaryText;
            routingStatusValueLabel.ForeColor = DarkSecondaryText;
            latencyStatusValueLabel.ForeColor = DarkSecondaryText;
            pluginStatusLabel.ForeColor = DarkSecondaryText;
            copyrightLabel.ForeColor = Color.FromArgb(104, 148, 153);
            copyrightLabel.Cursor = Cursors.Hand;
        }

        private static void ApplyDarkThemeRecursive(Control control)
        {
            switch (control)
            {
                case Panel panel:
                    panel.BackColor = DarkAppBackground;
                    panel.ForeColor = DarkPrimaryText;
                    break;

                case GroupBox groupBox:
                    groupBox.BackColor = DarkSurface;
                    groupBox.ForeColor = DarkPrimaryText;
                    groupBox.Paint -= DrawDarkGroupBox;
                    groupBox.Paint += DrawDarkGroupBox;
                    break;

                case Button button:
                    button.UseVisualStyleBackColor = false;
                    button.FlatStyle = FlatStyle.Flat;
                    button.BackColor = DarkButtonBackground;
                    button.ForeColor = DarkPrimaryText;
                    button.FlatAppearance.BorderSize = 0;
                    button.FlatAppearance.BorderColor = DarkSoftBorder;
                    button.FlatAppearance.MouseOverBackColor = DarkButtonHover;
                    button.FlatAppearance.MouseDownBackColor = DarkButtonPressed;
                    button.Paint -= DrawDarkButton;
                    button.Paint += DrawDarkButton;
                    button.EnabledChanged -= InvalidateDarkButton;
                    button.EnabledChanged += InvalidateDarkButton;
                    button.MouseEnter -= InvalidateDarkButton;
                    button.MouseEnter += InvalidateDarkButton;
                    button.MouseLeave -= InvalidateDarkButton;
                    button.MouseLeave += InvalidateDarkButton;
                    button.MouseDown -= InvalidateDarkButton;
                    button.MouseDown += InvalidateDarkButton;
                    button.MouseUp -= InvalidateDarkButton;
                    button.MouseUp += InvalidateDarkButton;
                    break;

                case TextBox textBox:
                    textBox.BackColor = DarkControlBackground;
                    textBox.ForeColor = DarkPrimaryText;
                    textBox.BorderStyle = BorderStyle.FixedSingle;
                    break;

                case ComboBox comboBox:
                    comboBox.BackColor = DarkControlBackground;
                    comboBox.ForeColor = DarkPrimaryText;
                    comboBox.FlatStyle = FlatStyle.Flat;
                    comboBox.DrawMode = DrawMode.OwnerDrawFixed;
                    if (comboBox is DarkComboBox darkComboBox)
                    {
                        darkComboBox.ApplyTheme(
                            DarkControlBackground,
                            DarkPrimaryText,
                            DarkAccentMint,
                            DarkOnAccent,
                            DarkSoftBorder,
                            DarkSecondaryText,
                            DarkCornerRadius);
                    }
                    else
                    {
                        comboBox.DrawItem -= DrawDarkComboBoxItem;
                        comboBox.DrawItem += DrawDarkComboBoxItem;
                    }
                    break;

                case ListBox listBox:
                    listBox.BackColor = DarkControlBackground;
                    listBox.ForeColor = DarkPrimaryText;
                    listBox.BorderStyle = BorderStyle.None;
                    break;

                case Label label:
                    label.BackColor = Color.Transparent;
                    label.ForeColor = DarkPrimaryText;
                    break;
            }

            foreach (Control child in control.Controls)
            {
                ApplyDarkThemeRecursive(child);
            }
        }

        private static void InvalidateDarkButton(object? sender, EventArgs e)
        {
            if (sender is Control control)
            {
                control.Invalidate();
            }
        }

        private static void DrawDarkButton(object? sender, PaintEventArgs e)
        {
            if (sender is not Button button)
            {
                return;
            }

            var style = button.Tag as string;
            var isPrimary = string.Equals(style, "primary", StringComparison.Ordinal);
            var isDanger = string.Equals(style, "danger", StringComparison.Ordinal);
            var isDangerOutline = string.Equals(style, "danger-outline", StringComparison.Ordinal);

            var hovered = button.Enabled &&
                button.ClientRectangle.Contains(button.PointToClient(Cursor.Position));
            var pressed = hovered && (Control.MouseButtons & MouseButtons.Left) == MouseButtons.Left;

            Color background;
            Color foreground;
            Color border;

            if (!button.Enabled)
            {
                background = DarkButtonDisabled;
                foreground = DarkButtonDisabledText;
                border = DarkBorder;
            }
            else if (isPrimary)
            {
                background = pressed
                    ? Color.FromArgb(0, 214, 164)
                    : hovered ? Color.FromArgb(64, 255, 210) : DarkAccentMint;
                foreground = DarkOnAccent;
                border = background;
            }
            else if (isDanger)
            {
                background = pressed
                    ? Color.FromArgb(224, 58, 100)
                    : hovered ? Color.FromArgb(255, 112, 149) : DarkDanger;
                foreground = Color.FromArgb(40, 4, 15);
                border = background;
            }
            else if (isDangerOutline)
            {
                background = pressed
                    ? Color.FromArgb(70, 27, 42)
                    : hovered ? Color.FromArgb(52, 21, 34) : DarkButtonBackground;
                foreground = DarkDanger;
                border = hovered ? DarkDanger : Color.FromArgb(102, 50, 68);
            }
            else
            {
                background = pressed
                    ? DarkButtonPressed
                    : hovered ? DarkButtonHover : DarkButtonBackground;
                foreground = DarkPrimaryText;
                border = hovered ? DarkSoftBorder : DarkBorder;
            }

            e.Graphics.SmoothingMode = SmoothingMode.AntiAlias;
            e.Graphics.Clear(button.Parent?.BackColor ?? DarkAppBackground);

            using var backgroundBrush = new SolidBrush(background);
            var borderBounds = new Rectangle(0, 0, button.Width - 1, button.Height - 1);
            using var buttonPath = CreateRoundedRectanglePath(borderBounds, DarkCornerRadius);
            e.Graphics.FillPath(backgroundBrush, buttonPath);

            using var borderPen = new Pen(border);
            e.Graphics.DrawPath(borderPen, buttonPath);

            TextRenderer.DrawText(
                e.Graphics,
                button.Text,
                button.Font,
                button.ClientRectangle,
                foreground,
                TextFormatFlags.HorizontalCenter |
                TextFormatFlags.VerticalCenter |
                TextFormatFlags.EndEllipsis |
                TextFormatFlags.NoPrefix);
        }

        private static void DrawDarkGroupBox(object? sender, PaintEventArgs e)
        {
            if (sender is not GroupBox groupBox)
            {
                return;
            }

            e.Graphics.SmoothingMode = SmoothingMode.AntiAlias;
            e.Graphics.Clear(groupBox.Parent?.BackColor ?? DarkAppBackground);

            var textSize = TextRenderer.MeasureText(groupBox.Text, groupBox.Font);
            var borderTop = Math.Max(8, textSize.Height / 2);
            var borderBounds = new Rectangle(
                0,
                borderTop,
                Math.Max(0, groupBox.Width - 1),
                Math.Max(0, groupBox.Height - borderTop - 1));

            using var borderPath = CreateRoundedRectanglePath(borderBounds, DarkCornerRadius);
            using var groupBackgroundBrush = new SolidBrush(groupBox.BackColor);
            e.Graphics.FillPath(groupBackgroundBrush, borderPath);

            using var borderPen = new Pen(DarkSoftBorder);
            e.Graphics.DrawPath(borderPen, borderPath);

            var textBounds = new Rectangle(8, 0, textSize.Width + 8, textSize.Height);
            using var textBackgroundBrush = new SolidBrush(groupBox.BackColor);
            e.Graphics.FillRectangle(textBackgroundBrush, textBounds);

            TextRenderer.DrawText(
                e.Graphics,
                groupBox.Text,
                groupBox.Font,
                new Rectangle(10, 0, textSize.Width, textSize.Height),
                DarkPrimaryText,
                TextFormatFlags.Left |
                TextFormatFlags.VerticalCenter |
                TextFormatFlags.NoPrefix);
        }

        private static GraphicsPath CreateRoundedRectanglePath(Rectangle bounds, int radius)
        {
            var path = new GraphicsPath();
            var diameter = Math.Max(1, radius * 2);

            if (bounds.Width <= diameter || bounds.Height <= diameter)
            {
                path.AddRectangle(bounds);
                path.CloseFigure();
                return path;
            }

            path.AddArc(bounds.Left, bounds.Top, diameter, diameter, 180, 90);
            path.AddArc(bounds.Right - diameter, bounds.Top, diameter, diameter, 270, 90);
            path.AddArc(bounds.Right - diameter, bounds.Bottom - diameter, diameter, diameter, 0, 90);
            path.AddArc(bounds.Left, bounds.Bottom - diameter, diameter, diameter, 90, 90);
            path.CloseFigure();

            return path;
        }

        private static void DrawDarkComboBoxItem(object? sender, DrawItemEventArgs e)
        {
            if (sender is not ComboBox comboBox)
            {
                return;
            }

            var isSelected = (e.State & DrawItemState.Selected) == DrawItemState.Selected;
            var isDisabled = (e.State & DrawItemState.Disabled) == DrawItemState.Disabled || !comboBox.Enabled;
            var background = isSelected && !isDisabled
                ? DarkAccentMint
                : DarkControlBackground;
            var foreground = isDisabled
                ? DarkSecondaryText
                : isSelected ? DarkOnAccent : DarkPrimaryText;

            using var backgroundBrush = new SolidBrush(background);
            e.Graphics.FillRectangle(backgroundBrush, e.Bounds);

            var text = e.Index >= 0 && e.Index < comboBox.Items.Count
                ? comboBox.GetItemText(comboBox.Items[e.Index])
                : comboBox.Text;

            var textBounds = new Rectangle(
                e.Bounds.Left + 4,
                e.Bounds.Top,
                Math.Max(0, e.Bounds.Width - 8),
                e.Bounds.Height);

            TextRenderer.DrawText(
                e.Graphics,
                text,
                comboBox.Font,
                textBounds,
                foreground,
                TextFormatFlags.VerticalCenter |
                TextFormatFlags.Left |
                TextFormatFlags.EndEllipsis |
                TextFormatFlags.NoPrefix);

            if ((e.State & DrawItemState.Focus) == DrawItemState.Focus)
            {
                using var focusPen = new Pen(DarkBorder);
                var focusBounds = Rectangle.Inflate(e.Bounds, -1, -1);
                e.Graphics.DrawRectangle(focusPen, focusBounds);
            }
        }

        private void RefreshButton_Click(object? sender, EventArgs e)
        {
            StopAudioRoute("Stopped: devices refreshed");
            RefreshAudioDevices();
        }

        private void InputDeviceComboBox_SelectedIndexChanged(object? sender, EventArgs e)
        {
            if (_isRefreshingDevices)
            {
                return;
            }

            StopAudioRoute("Stopped: input changed");
            StartInputLevelMonitor(inputDeviceComboBox.SelectedItem as AudioInputDevice);
        }

        private void OutputDeviceComboBox_SelectedIndexChanged(object? sender, EventArgs e)
        {
            if (_isRefreshingDevices)
            {
                return;
            }

            StopAudioRoute("Stopped: output changed");
            UpdateOutputSelectionStatus();
        }

        private void LevelTimer_Tick(object? sender, EventArgs e)
        {
            inputLevelMeter.Level = _audioRoutingService.IsRunning
                ? _audioRoutingService.GetInputPeakLevel()
                : _inputLevelMonitor?.GetPeakLevel() ?? 0;
            outputLevelMeter.Level = _audioRoutingService.GetOutputPeakLevel();
            UpdateRunningRouteStatus();
            UpdateLatencyStatus();
        }

        private void StartButton_Click(object? sender, EventArgs e)
        {
            TryStartAudioRouteFromCurrentSelection();
        }

        private void StopButton_Click(object? sender, EventArgs e)
        {
            StopAudioRoute("Stopped");
        }

        private void BrowsePluginFolderButton_Click(object? sender, EventArgs e)
        {
            using var dialog = new FolderBrowserDialog
            {
                Description = "Select VST plugin folder",
                UseDescriptionForTitle = true,
                SelectedPath = Directory.Exists(pluginFolderTextBox.Text)
                    ? pluginFolderTextBox.Text
                    : GetDefaultPluginFolder(),
            };

            if (dialog.ShowDialog(this) != DialogResult.OK)
            {
                return;
            }

            pluginFolderTextBox.Text = dialog.SelectedPath;
            RefreshPluginList();
        }

        private void scanPluginsButton_Click(object? sender, EventArgs e)
        {
            RefreshPluginList();
        }

        private void addPluginButton_Click(object? sender, EventArgs e)
        {
            if (foundPluginsListBox.SelectedItem is not VstPluginCandidate plugin)
            {
                pluginStatusLabel.Text = "Select a plugin first";
                return;
            }

            IAudioPluginHost? host = null;
            var wasRunning = _audioRoutingService.IsRunning;
            var stoppedForRestart = false;

            try
            {
                host = CreatePluginHost(plugin);

                if (wasRunning)
                {
                    StopAudioRouteForInternalRestart();
                    stoppedForRestart = true;
                }

                var chainItem = new VstPluginChainItem(plugin.Name, plugin.Path, plugin.Format, host);
                _isUpdatingPluginChainChecks = true;
                try
                {
                    pluginChainListBox.Items.Add(chainItem, chainItem.IsEnabled);
                }
                finally
                {
                    _isUpdatingPluginChainChecks = false;
                }

                host = null;

                if (wasRunning)
                {
                    TryRestartAudioRouteAfterChainChange($"Added: {chainItem}");
                }
                else
                {
                    pluginStatusLabel.Text = $"Added: {chainItem}";
                }
            }
            catch (NativeVstHostException ex)
            {
                RecoverFromFailedChainChange($"Add failed: {ex.Message}", stoppedForRestart);
            }
            catch (Exception ex)
            {
                RecoverFromFailedChainChange($"Add failed: {ex.Message}", stoppedForRestart);
            }
            finally
            {
                host?.Dispose();
            }

            UpdatePluginChainButtons();
        }

        private static IAudioPluginHost CreatePluginHost(VstPluginCandidate plugin)
        {
            return CreatePluginHost(plugin.Path, plugin.Format);
        }

        private static IAudioPluginHost CreatePluginHost(string pluginPath, VstPluginFormat format)
        {
            IAudioPluginHost host = format switch
            {
                VstPluginFormat.Vst2 => new NativeVst2Host(),
                _ => new NativeVstHost(),
            };

            try
            {
                host.LoadPlugin(pluginPath);
                return host;
            }
            catch
            {
                host.Dispose();
                throw;
            }
        }

        private void removePluginButton_Click(object? sender, EventArgs e)
        {
            var selectedIndex = pluginChainListBox.SelectedIndex;
            if (selectedIndex < 0)
            {
                pluginStatusLabel.Text = "Select a chain plugin first";
                return;
            }

            var chainItem = pluginChainListBox.SelectedItem as VstPluginChainItem;
            var pluginName = chainItem?.Name ?? pluginChainListBox.SelectedItem?.ToString() ?? "plugin";
            var wasRunning = _audioRoutingService.IsRunning;

            if (wasRunning)
            {
                StopAudioRouteForInternalRestart();
            }

            if (chainItem is not null)
            {
                ClosePluginEditor(chainItem);
                chainItem.Dispose();
            }

            pluginChainListBox.Items.RemoveAt(selectedIndex);

            if (pluginChainListBox.Items.Count > 0)
            {
                pluginChainListBox.SelectedIndex = Math.Min(selectedIndex, pluginChainListBox.Items.Count - 1);
            }

            if (wasRunning)
            {
                TryRestartAudioRouteAfterChainChange($"Removed: {pluginName}");
            }
            else
            {
                pluginStatusLabel.Text = $"Removed: {pluginName}";
            }

            UpdatePluginChainButtons();
        }

        private void movePluginUpButton_Click(object? sender, EventArgs e)
        {
            MoveSelectedPlugin(-1);
        }

        private void movePluginDownButton_Click(object? sender, EventArgs e)
        {
            MoveSelectedPlugin(1);
        }

        private void openPluginEditorButton_Click(object? sender, EventArgs e)
        {
            if (pluginChainListBox.SelectedItem is not VstPluginChainItem plugin)
            {
                pluginStatusLabel.Text = "Select a chain plugin first";
                return;
            }

            OpenPluginEditor(plugin);
        }

        private void OpenPluginEditor(VstPluginChainItem plugin)
        {
            if (_pluginEditorForms.TryGetValue(plugin, out var existingEditor))
            {
                if (!existingEditor.IsDisposed)
                {
                    existingEditor.Activate();
                    return;
                }

                _pluginEditorForms.Remove(plugin);
            }

            var editorForm = new Form
            {
                Text = $"{plugin.Name} editor",
                StartPosition = FormStartPosition.CenterParent,
                Size = new Size(900, 650),
                MinimumSize = new Size(480, 320),
                MinimizeBox = false,
                FormBorderStyle = FormBorderStyle.FixedSingle,
                MaximizeBox = false,
                ShowInTaskbar = false,
                BackColor = DarkAppBackground,
                ForeColor = DarkPrimaryText,
            };

            var editorOpened = false;
            var editorIdleTimer = new System.Windows.Forms.Timer
            {
                Interval = 33,
            };
            _pluginEditorForms[plugin] = editorForm;

            editorForm.Shown += (_, _) =>
            {
                try
                {
                    plugin.OpenEditor(editorForm.Handle);
                    ApplyPreferredEditorSize(editorForm, plugin);
                    if (plugin.Format == VstPluginFormat.Vst2)
                    {
                        editorIdleTimer.Start();
                    }

                    editorOpened = true;
                    pluginStatusLabel.Text = $"Editor opened: {plugin.Name}";
                }
                catch (NativeVstHostException ex)
                {
                    editorIdleTimer.Stop();
                    ShowEditorError(editorForm, ex.Message, plugin.Path);
                    pluginStatusLabel.Text = ex.Message;
                }
                catch (Exception ex)
                {
                    editorIdleTimer.Stop();
                    ShowEditorError(editorForm, ex.Message, plugin.Path);
                    pluginStatusLabel.Text = ex.Message;
                }
            };

            editorIdleTimer.Tick += (_, _) =>
            {
                try
                {
                    plugin.EditorIdle();
                }
                catch (Exception ex)
                {
                    editorIdleTimer.Stop();
                    pluginStatusLabel.Text = $"Editor idle failed: {ex.Message}";
                }
            };

            editorForm.FormClosed += (_, _) =>
            {
                editorIdleTimer.Stop();
                editorIdleTimer.Dispose();
                _pluginEditorForms.Remove(plugin);

                if (!editorOpened)
                {
                    return;
                }

                try
                {
                    plugin.CloseEditor();
                }
                catch (NativeVstHostException ex)
                {
                    pluginStatusLabel.Text = ex.Message;
                }
                finally
                {
                    editorOpened = false;
                }
            };

            editorForm.Show(this);
        }

        private static void ApplyPreferredEditorSize(Form editorForm, VstPluginChainItem plugin)
        {
            var preferredSize = plugin.GetEditorSize();
            if (preferredSize is not { Width: > 0, Height: > 0 } size)
            {
                return;
            }

            var workingArea = Screen.FromControl(editorForm).WorkingArea;
            var maxClientWidth = Math.Max(320, workingArea.Width - 120);
            var maxClientHeight = Math.Max(240, workingArea.Height - 120);
            editorForm.ClientSize = new Size(
                Math.Clamp(size.Width, 320, maxClientWidth),
                Math.Clamp(size.Height, 220, maxClientHeight));
        }

        private static void ShowEditorError(Form editorForm, string message, string pluginPath)
        {
            editorForm.Controls.Clear();
            editorForm.Controls.Add(new Label
            {
                AutoEllipsis = true,
                BackColor = DarkAppBackground,
                ForeColor = DarkPrimaryText,
                Dock = DockStyle.Fill,
                Padding = new Padding(18),
                Text = $"{message}\r\n\r\n{pluginPath}",
            });
        }

        private void pluginChainListBox_SelectedIndexChanged(object? sender, EventArgs e)
        {
            UpdatePluginChainButtons();
        }

        private void pluginChainListBox_ItemCheck(object? sender, ItemCheckEventArgs e)
        {
            if (_isUpdatingPluginChainChecks)
            {
                return;
            }

            if (!_isPluginChainCheckboxClick)
            {
                e.NewValue = e.CurrentValue;
                return;
            }

            if (e.Index < 0 || pluginChainListBox.Items[e.Index] is not VstPluginChainItem chainItem)
            {
                return;
            }

            chainItem.IsEnabled = e.NewValue == CheckState.Checked;

            var wasRunning = _audioRoutingService.IsRunning;
            if (wasRunning)
            {
                StopAudioRouteForInternalRestart();
                TryRestartAudioRouteAfterChainChange(chainItem.IsEnabled
                    ? $"Enabled: {chainItem.Name}"
                    : $"Bypassed: {chainItem.Name}");
            }
            else
            {
                pluginStatusLabel.Text = chainItem.IsEnabled
                    ? $"Enabled: {chainItem.Name}"
                    : $"Bypassed: {chainItem.Name}";
            }
        }

        private void pluginChainListBox_MouseDown(object? sender, MouseEventArgs e)
        {
            var clickedIndex = pluginChainListBox.IndexFromPoint(e.Location);
            if (clickedIndex < 0)
            {
                return;
            }

            pluginChainListBox.SelectedIndex = clickedIndex;

            if (e.Button != MouseButtons.Left || !IsPluginChainCheckBoxHit(e.Location))
            {
                return;
            }

            if (e.Clicks > 1)
            {
                return;
            }

            _isPluginChainCheckboxClick = true;
            try
            {
                pluginChainListBox.SetItemChecked(clickedIndex, !pluginChainListBox.GetItemChecked(clickedIndex));
            }
            finally
            {
                _isPluginChainCheckboxClick = false;
            }
        }

        private void pluginChainListBox_MouseDoubleClick(object? sender, MouseEventArgs e)
        {
            var clickedIndex = pluginChainListBox.IndexFromPoint(e.Location);
            if (clickedIndex < 0)
            {
                return;
            }

            if (IsPluginChainCheckBoxHit(e.Location))
            {
                return;
            }

            pluginChainListBox.SelectedIndex = clickedIndex;
            openPluginEditorButton_Click(sender, EventArgs.Empty);
        }

        private bool IsPluginChainCheckBoxHit(Point location)
        {
            var clickedIndex = pluginChainListBox.IndexFromPoint(location);
            if (clickedIndex < 0)
            {
                return false;
            }

            var itemBounds = pluginChainListBox.GetItemRectangle(clickedIndex);
            var checkBoxWidth = SystemInformation.MenuCheckSize.Width + 8;
            var checkBoxBounds = new Rectangle(
                itemBounds.Left,
                itemBounds.Top,
                checkBoxWidth,
                itemBounds.Height);

            return checkBoxBounds.Contains(location);
        }

        private void foundPluginsListBox_SelectedIndexChanged(object? sender, EventArgs e)
        {
            UpdatePluginChainButtons();
        }

        private void foundPluginsListBox_MouseDoubleClick(object? sender, MouseEventArgs e)
        {
            var clickedIndex = foundPluginsListBox.IndexFromPoint(e.Location);
            if (clickedIndex < 0)
            {
                return;
            }

            foundPluginsListBox.SelectedIndex = clickedIndex;
            addPluginButton_Click(sender, EventArgs.Empty);
        }

        private void RefreshAudioDevices()
        {
            var selectedInputDeviceId = (inputDeviceComboBox.SelectedItem as AudioInputDevice)?.Id;
            var selectedOutputDeviceId = (outputDeviceComboBox.SelectedItem as AudioOutputDevice)?.Id;
            var inputDevices = _audioInputDeviceScanner.GetInputDevices();
            var outputDevices = _audioOutputDeviceScanner.GetOutputDevices();

            _isRefreshingDevices = true;
            PopulateInputDevices(inputDevices, selectedInputDeviceId);
            PopulateOutputDevices(outputDevices, selectedOutputDeviceId);
            _isRefreshingDevices = false;

            UpdateVirtualCableStatus(inputDevices, outputDevices);
            StartInputLevelMonitor(inputDeviceComboBox.SelectedItem as AudioInputDevice);
            UpdateOutputSelectionStatus();
        }

        private void InitializeBufferSizeComboBox()
        {
            bufferSizeComboBox.Items.Clear();
            foreach (var size in new[] { 128, 256, 512, 1024, 2048, 4096 })
            {
                bufferSizeComboBox.Items.Add(size);
            }

            bufferSizeComboBox.SelectedItem = 512;
        }

        private int GetSelectedBufferSize()
        {
            return bufferSizeComboBox.SelectedItem is int size
                ? size
                : 512;
        }

        private void PopulateInputDevices(IReadOnlyList<AudioInputDevice> inputDevices, string? selectedDeviceId)
        {
            inputDeviceComboBox.BeginUpdate();
            inputDeviceComboBox.DataSource = null;
            inputDeviceComboBox.Items.Clear();

            if (inputDevices.Count == 0)
            {
                inputDeviceComboBox.Items.Add("No input devices found");
                inputDeviceComboBox.SelectedIndex = 0;
                inputDeviceComboBox.Enabled = false;
            }
            else
            {
                var inputDeviceList = inputDevices.ToList();
                inputDeviceComboBox.DataSource = inputDeviceList;
                inputDeviceComboBox.DropDownWidth = GetPreferredDropDownWidth(inputDeviceComboBox, inputDeviceList);
                var selectedIndex = Math.Max(0, inputDeviceList.FindIndex(device => device.Id == selectedDeviceId));
                inputDeviceComboBox.SelectedIndex = selectedIndex;
                inputDeviceComboBox.Enabled = true;
            }

            inputDeviceComboBox.EndUpdate();
        }

        private void PopulateOutputDevices(IReadOnlyList<AudioOutputDevice> outputDevices, string? selectedDeviceId)
        {
            outputDeviceComboBox.BeginUpdate();
            outputDeviceComboBox.DataSource = null;
            outputDeviceComboBox.Items.Clear();

            if (outputDevices.Count == 0)
            {
                outputDeviceComboBox.Items.Add("No output devices found");
                outputDeviceComboBox.SelectedIndex = 0;
                outputDeviceComboBox.Enabled = false;
            }
            else
            {
                var outputDeviceList = outputDevices.ToList();
                outputDeviceComboBox.DataSource = outputDeviceList;
                outputDeviceComboBox.DropDownWidth = GetPreferredDropDownWidth(outputDeviceComboBox, outputDeviceList);
                var selectedIndex = outputDeviceList.FindIndex(device => device.Id == selectedDeviceId);

                if (selectedIndex < 0)
                {
                    var preferredOutputDevice = _virtualCableService.FindPreferredOutputDevice(outputDeviceList);
                    selectedIndex = preferredOutputDevice is null
                        ? 0
                        : outputDeviceList.FindIndex(device => device.Id == preferredOutputDevice.Id);
                }

                outputDeviceComboBox.SelectedIndex = Math.Max(0, selectedIndex);
                outputDeviceComboBox.Enabled = true;
            }

            outputDeviceComboBox.EndUpdate();
        }

        private void UpdateVirtualCableStatus(
            IReadOnlyList<AudioInputDevice> inputDevices,
            IReadOnlyList<AudioOutputDevice> outputDevices)
        {
            var virtualCableStatus = _virtualCableService.GetStatus(inputDevices, outputDevices);
            cableOutputValueLabel.Text = virtualCableStatus.OutputDeviceName;
            cableInputValueLabel.Text = virtualCableStatus.InputDeviceName;
            cableStateValueLabel.Text = virtualCableStatus.Message;
            cableStateValueLabel.ForeColor = virtualCableStatus.IsReady
                ? DarkAccentMint
                : DarkDanger;
        }

        private static int GetPreferredDropDownWidth<T>(ComboBox comboBox, IReadOnlyList<T> devices)
        {
            if (devices.Count == 0)
            {
                return comboBox.Width;
            }

            var maxTextWidth = devices
                .Select(device => TextRenderer.MeasureText(device?.ToString() ?? string.Empty, comboBox.Font).Width)
                .Max();

            return Math.Max(comboBox.Width, maxTextWidth + SystemInformation.VerticalScrollBarWidth + 24);
        }

        private void StartInputLevelMonitor(AudioInputDevice? inputDevice)
        {
            StopInputLevelMonitor();

            if (inputDevice is null)
            {
                inputLevelStatusLabel.Text = "No input selected";
                return;
            }

            try
            {
                _inputLevelMonitor = new AudioInputLevelMonitor(inputDevice);
                inputLevelStatusLabel.Text = inputDevice.Name;
            }
            catch
            {
                inputLevelStatusLabel.Text = "Meter unavailable";
            }
        }

        private void StopInputLevelMonitor()
        {
            _inputLevelMonitor?.Dispose();
            _inputLevelMonitor = null;
            inputLevelMeter.Level = 0;
        }

        private void UpdateOutputSelectionStatus()
        {
            var outputDevice = outputDeviceComboBox.SelectedItem as AudioOutputDevice;
            outputLevelStatusLabel.Text = outputDevice?.Name ?? "No output selected";
        }

        private void StopAudioRoute(string status)
        {
            _audioRoutingService.Stop();
            outputLevelMeter.Level = 0;
            startButton.Enabled = true;
            stopButton.Enabled = false;
            _lastAudioProcessingStatus = string.Empty;
            routingStatusValueLabel.Text = status;
            routingStatusValueLabel.ForeColor = DarkSecondaryText;
            UpdateLatencyStatus();
            UpdateOutputSelectionStatus();
            StartInputLevelMonitor(inputDeviceComboBox.SelectedItem as AudioInputDevice);
        }

        private void StopAudioRouteForInternalRestart()
        {
            _audioRoutingService.Stop();
            outputLevelMeter.Level = 0;
            _lastAudioProcessingStatus = string.Empty;
        }

        private bool TryRestartAudioRouteAfterChainChange(string actionStatus)
        {
            var restarted = TryStartAudioRouteFromCurrentSelection();
            pluginStatusLabel.Text = restarted
                ? actionStatus
                : $"Restart failed after chain change: {pluginStatusLabel.Text}";

            return restarted;
        }

        private void RecoverFromFailedChainChange(string failureStatus, bool routeWasStoppedForRestart)
        {
            if (!routeWasStoppedForRestart)
            {
                pluginStatusLabel.Text = failureStatus;
                return;
            }

            var restarted = TryStartAudioRouteFromCurrentSelection();
            pluginStatusLabel.Text = restarted
                ? failureStatus
                : $"{failureStatus}. Route restart failed: {pluginStatusLabel.Text}";
        }

        private bool TryStartAudioRouteFromCurrentSelection()
        {
            var inputDevice = inputDeviceComboBox.SelectedItem as AudioInputDevice;
            var outputDevice = outputDeviceComboBox.SelectedItem as AudioOutputDevice;

            if (inputDevice is null || outputDevice is null)
            {
                routingStatusValueLabel.Text = "Select input and output";
                routingStatusValueLabel.ForeColor = DarkDanger;
                startButton.Enabled = true;
                stopButton.Enabled = false;
                return false;
            }

            try
            {
                StopInputLevelMonitor();
                _audioRoutingService.Start(
                    inputDevice,
                    outputDevice,
                    GetPluginChainSnapshot(),
                    GetSelectedBufferSize());
                startButton.Enabled = false;
                stopButton.Enabled = true;
                UpdateRunningRouteStatus(force: true);
                outputLevelStatusLabel.Text = outputDevice.Name;
                return true;
            }
            catch (Exception ex)
            {
                StopAudioRoute("Error");
                routingStatusValueLabel.Text = ex.Message;
                routingStatusValueLabel.ForeColor = DarkDanger;
                pluginStatusLabel.Text = ex.Message;
                return false;
            }
        }

        private void UpdateRunningRouteStatus(bool force = false)
        {
            if (!_audioRoutingService.IsRunning)
            {
                return;
            }

            var processingStatus = _audioRoutingService.ProcessingStatus;
            if (!force && processingStatus == _lastAudioProcessingStatus)
            {
                return;
            }

            _lastAudioProcessingStatus = processingStatus;
            routingStatusValueLabel.Text = $"Running - {processingStatus}";
            routingStatusValueLabel.ForeColor = _audioRoutingService.IsVstProcessingActive
                ? DarkAccentMint
                : DarkAccentTeal;
            pluginStatusLabel.Text = processingStatus;
        }

        private void UpdateLatencyStatus()
        {
            var diagnostics = _audioRoutingService.GetDiagnostics();
            latencyStatusValueLabel.Text =
                $"q/o/c/p {diagnostics.BufferedMs:0}/{diagnostics.RequestedOutputLatencyMs}/{diagnostics.LastCaptureBlockMs:0}/{diagnostics.InitialPreloadMs:0}ms";
        }

        private void RefreshPluginList()
        {
            var plugins = _vstPluginScanner.Scan(pluginFolderTextBox.Text);

            foundPluginsListBox.BeginUpdate();
            foundPluginsListBox.Items.Clear();

            foreach (var plugin in plugins)
            {
                foundPluginsListBox.Items.Add(plugin);
            }

            foundPluginsListBox.EndUpdate();
            pluginStatusLabel.Text = GetPluginStatusText(plugins);
            UpdatePluginChainButtons();
        }

        private void UpdateNativeVstStatus()
        {
            var statusParts = new List<string>();

            try
            {
                statusParts.Add($"VST3 API v{NativeVstHost.ApiVersion}");
            }
            catch (NativeVstHostException ex)
            {
                statusParts.Add(ex.Message);
            }
            catch (Exception ex)
            {
                statusParts.Add($"Native VST3 host unavailable: {ex.Message}");
            }

            try
            {
                statusParts.Add($"VST2 API v{NativeVst2Host.ApiVersion}");
            }
            catch (NativeVstHostException ex)
            {
                statusParts.Add(ex.Message);
            }
            catch (Exception ex)
            {
                statusParts.Add($"Native VST2 host unavailable: {ex.Message}");
            }

            _nativeVstStatus = string.Join(" / ", statusParts);
        }

        private string GetPluginStatusText(IReadOnlyCollection<VstPluginCandidate> plugins)
        {
            var vst3Count = plugins.Count(plugin => plugin.Format == VstPluginFormat.Vst3);
            var vst2Count = plugins.Count(plugin => plugin.Format == VstPluginFormat.Vst2);
            var pluginScanStatus = plugins.Count == 0
                ? "No VST plugins found"
                : $"Found {vst3Count} VST3 + {vst2Count} VST2 x64 plugin(s)";

            return $"{_nativeVstStatus}. {pluginScanStatus}";
        }

        private static string GetDefaultPluginFolder()
        {
            var outputPluginFolder = Path.Combine(AppContext.BaseDirectory, "common", "VST");
            if (Directory.Exists(outputPluginFolder))
            {
                return outputPluginFolder;
            }

            var repoPluginFolder = Path.GetFullPath(Path.Combine(
                AppContext.BaseDirectory,
                "..",
                "..",
                "..",
                "..",
                "common",
                "VST"));

            return Directory.Exists(repoPluginFolder)
                ? repoPluginFolder
                : outputPluginFolder;
        }

        private void UpdatePluginChainButtons()
        {
            var selectedIndex = pluginChainListBox.SelectedIndex;
            var hasChainSelection = pluginChainListBox.SelectedItem is VstPluginChainItem;
            movePluginUpButton.Enabled = hasChainSelection && selectedIndex > 0;
            movePluginDownButton.Enabled = hasChainSelection && selectedIndex < pluginChainListBox.Items.Count - 1;
            removePluginButton.Enabled = hasChainSelection;
            openPluginEditorButton.Enabled = hasChainSelection;
            addPluginButton.Enabled = foundPluginsListBox.SelectedItem is VstPluginCandidate;
        }

        private void MoveSelectedPlugin(int direction)
        {
            var selectedIndex = pluginChainListBox.SelectedIndex;
            var targetIndex = selectedIndex + direction;

            if (selectedIndex < 0 ||
                targetIndex < 0 ||
                targetIndex >= pluginChainListBox.Items.Count ||
                pluginChainListBox.SelectedItem is not VstPluginChainItem chainItem)
            {
                UpdatePluginChainButtons();
                return;
            }

            var wasRunning = _audioRoutingService.IsRunning;
            var wasChecked = pluginChainListBox.GetItemChecked(selectedIndex);
            if (wasRunning)
            {
                StopAudioRouteForInternalRestart();
            }

            _isUpdatingPluginChainChecks = true;
            try
            {
                pluginChainListBox.Items.RemoveAt(selectedIndex);
                pluginChainListBox.Items.Insert(targetIndex, chainItem);
                pluginChainListBox.SetItemChecked(targetIndex, wasChecked);
                pluginChainListBox.SelectedIndex = targetIndex;
            }
            finally
            {
                _isUpdatingPluginChainChecks = false;
            }

            if (wasRunning)
            {
                var directionName = direction < 0 ? "up" : "down";
                TryRestartAudioRouteAfterChainChange($"Moved {directionName}: {chainItem.Name}");
            }
            else
            {
                var directionName = direction < 0 ? "up" : "down";
                pluginStatusLabel.Text = $"Moved {directionName}: {chainItem.Name}";
            }

            UpdatePluginChainButtons();
        }

        private IReadOnlyList<VstPluginChainItem> GetPluginChainSnapshot()
        {
            var pluginChain = new List<VstPluginChainItem>();

            foreach (var item in pluginChainListBox.Items)
            {
                if (item is VstPluginChainItem { IsEnabled: true } chainItem)
                {
                    pluginChain.Add(chainItem);
                }
            }

            return pluginChain;
        }

        private void ApplySessionPreferences(VoiceSetupProfile? profile)
        {
            if (profile is null)
            {
                return;
            }

            ApplyDevicePreferences(profile);
            ApplyBufferSize(profile.BufferSize);
        }

        private void ApplyDevicePreferences(VoiceSetupProfile profile)
        {
            var inputDevices = inputDeviceComboBox.Items.OfType<AudioInputDevice>().ToList();
            var inputMatch = inputDevices.FirstOrDefault(device => device.Id == profile.InputDeviceId)
                ?? inputDevices.FirstOrDefault(device => string.Equals(
                    device.Name,
                    profile.InputDeviceName,
                    StringComparison.OrdinalIgnoreCase));
            if (inputMatch is not null)
            {
                inputDeviceComboBox.SelectedItem = inputMatch;
            }

            var outputDevices = outputDeviceComboBox.Items.OfType<AudioOutputDevice>().ToList();
            var outputMatch = outputDevices.FirstOrDefault(device => device.Id == profile.OutputDeviceId)
                ?? outputDevices.FirstOrDefault(device => string.Equals(
                    device.Name,
                    profile.OutputDeviceName,
                    StringComparison.OrdinalIgnoreCase));
            if (outputMatch is not null)
            {
                outputDeviceComboBox.SelectedItem = outputMatch;
            }
        }

        private void ApplyBufferSize(int bufferSize)
        {
            if (bufferSize > 0 && bufferSizeComboBox.Items.Contains(bufferSize))
            {
                bufferSizeComboBox.SelectedItem = bufferSize;
            }
        }

        private (int RestoredCount, List<string> Failures) RestorePluginChainFromProfile(VoiceSetupProfile? profile)
        {
            var failures = new List<string>();
            var restoredCount = 0;

            if (profile?.Plugins is not { Count: > 0 } pluginEntries)
            {
                return (restoredCount, failures);
            }

            _isUpdatingPluginChainChecks = true;
            try
            {
                foreach (var entry in pluginEntries)
                {
                    if (string.IsNullOrWhiteSpace(entry.Path) || !File.Exists(entry.Path))
                    {
                        failures.Add(entry.Name);
                        continue;
                    }

                    IAudioPluginHost? host = null;
                    try
                    {
                        var format = string.Equals(entry.Format, "Vst2", StringComparison.OrdinalIgnoreCase)
                            ? VstPluginFormat.Vst2
                            : VstPluginFormat.Vst3;
                        host = CreatePluginHost(entry.Path, format);

                        if (entry.State is { Length: > 0 } stateBase64)
                        {
                            try
                            {
                                host.LoadState(Convert.FromBase64String(stateBase64));
                            }
                            catch (Exception)
                            {
                                // The plugin still loads with its defaults when a saved
                                // state blob cannot be applied; keep the chain item.
                            }
                        }

                        var chainItem = new VstPluginChainItem(entry.Name, entry.Path, format, host);
                        host = null;
                        chainItem.IsEnabled = entry.Enabled;
                        pluginChainListBox.Items.Add(chainItem, entry.Enabled);
                        restoredCount++;
                    }
                    catch (Exception)
                    {
                        failures.Add(entry.Name);
                    }
                    finally
                    {
                        host?.Dispose();
                    }
                }
            }
            finally
            {
                _isUpdatingPluginChainChecks = false;
            }

            UpdatePluginChainButtons();
            return (restoredCount, failures);
        }

        private VoiceSetupProfile CaptureCurrentSetup(string name)
        {
            var inputDevice = inputDeviceComboBox.SelectedItem as AudioInputDevice;
            var outputDevice = outputDeviceComboBox.SelectedItem as AudioOutputDevice;

            var profile = new VoiceSetupProfile
            {
                Name = name,
                ActiveProfile = _activeProfileName,
                PluginFolder = pluginFolderTextBox.Text,
                InputDeviceId = inputDevice?.Id,
                InputDeviceName = inputDevice?.Name,
                OutputDeviceId = outputDevice?.Id,
                OutputDeviceName = outputDevice?.Name,
                BufferSize = GetSelectedBufferSize(),
            };

            foreach (var item in pluginChainListBox.Items)
            {
                if (item is not VstPluginChainItem chainItem)
                {
                    continue;
                }

                string? stateBase64 = null;
                try
                {
                    var stateBytes = chainItem.SaveState();
                    if (stateBytes is { Length: > 0 })
                    {
                        stateBase64 = Convert.ToBase64String(stateBytes);
                    }
                }
                catch (Exception)
                {
                    // State capture is best effort; the plugin is still persisted
                    // by path, order and enabled flag.
                }

                profile.Plugins.Add(new VoiceSetupPluginEntry
                {
                    Name = chainItem.Name,
                    Path = chainItem.Path,
                    Format = chainItem.Format == VstPluginFormat.Vst2 ? "Vst2" : "Vst3",
                    Enabled = chainItem.IsEnabled,
                    State = stateBase64,
                });
            }

            return profile;
        }

        private void RefreshProfileList(string? selectName)
        {
            _isRefreshingProfiles = true;
            try
            {
                profileComboBox.Items.Clear();
                foreach (var profileName in VoiceSetupStore.GetProfileNames())
                {
                    profileComboBox.Items.Add(profileName);
                }

                if (selectName is { Length: > 0 })
                {
                    var index = profileComboBox.FindStringExact(selectName);
                    if (index >= 0)
                    {
                        profileComboBox.SelectedIndex = index;
                    }
                }
            }
            finally
            {
                _isRefreshingProfiles = false;
            }

            UpdateProfileButtons();
        }

        private void UpdateProfileButtons()
        {
            deleteProfileButton.Enabled = profileComboBox.SelectedItem is string;
        }

        private void profileComboBox_SelectedIndexChanged(object? sender, EventArgs e)
        {
            if (_isRefreshingProfiles)
            {
                return;
            }

            if (profileComboBox.SelectedItem is not string profileName ||
                string.Equals(profileName, _activeProfileName, StringComparison.OrdinalIgnoreCase))
            {
                UpdateProfileButtons();
                return;
            }

            var profile = VoiceSetupStore.LoadProfile(profileName);
            if (profile is null)
            {
                pluginStatusLabel.Text = $"Profile not found: {profileName}";
                RefreshProfileList(_activeProfileName);
                return;
            }

            ApplyProfile(profile);
        }

        private void ApplyProfile(VoiceSetupProfile profile)
        {
            StopAudioRoute("Stopped: profile loading");
            CloseAllPluginEditors();
            DisposePluginChain();

            if (profile.PluginFolder is { Length: > 0 } pluginFolder && Directory.Exists(pluginFolder))
            {
                pluginFolderTextBox.Text = pluginFolder;
            }

            RefreshPluginList();
            ApplyDevicePreferences(profile);
            ApplyBufferSize(profile.BufferSize);

            var (restoredCount, failures) = RestorePluginChainFromProfile(profile);
            _activeProfileName = profile.Name;
            pluginStatusLabel.Text = failures.Count > 0
                ? $"Profile loaded: {profile.Name} ({restoredCount} plugin(s)). Skipped: {string.Join(", ", failures)}"
                : $"Profile loaded: {profile.Name} ({restoredCount} plugin(s))";
        }

        private void saveProfileButton_Click(object? sender, EventArgs e)
        {
            if (profileComboBox.SelectedItem is not string profileName)
            {
                CreateProfile();
                return;
            }

            SaveProfile(profileName);
        }

        private void createProfileButton_Click(object? sender, EventArgs e)
        {
            CreateProfile();
        }

        private void deleteProfileButton_Click(object? sender, EventArgs e)
        {
            if (profileComboBox.SelectedItem is not string profileName)
            {
                pluginStatusLabel.Text = "Select a profile to delete";
                return;
            }

            var confirm = MessageBox.Show(
                this,
                $"Delete profile \"{profileName}\"?",
                "Delete profile",
                MessageBoxButtons.YesNo,
                MessageBoxIcon.Warning);
            if (confirm != DialogResult.Yes)
            {
                return;
            }

            try
            {
                VoiceSetupStore.DeleteProfile(profileName);
                if (string.Equals(_activeProfileName, profileName, StringComparison.OrdinalIgnoreCase))
                {
                    _activeProfileName = null;
                }

                RefreshProfileList(null);
                pluginStatusLabel.Text = $"Profile deleted: {profileName}";
            }
            catch (Exception ex)
            {
                pluginStatusLabel.Text = $"Profile delete failed: {ex.Message}";
            }
        }

        private void CreateProfile()
        {
            var suggestedName = SuggestNewProfileName();

            using var dialog = new ProfileNameDialog(suggestedName);
            if (dialog.ShowDialog(this) != DialogResult.OK)
            {
                return;
            }

            var profileName = dialog.ProfileName;
            if (VoiceSetupStore.ProfileExists(profileName))
            {
                var confirm = MessageBox.Show(
                    this,
                    $"Profile \"{profileName}\" already exists. Overwrite it?",
                    "Create profile",
                    MessageBoxButtons.YesNo,
                    MessageBoxIcon.Question);
                if (confirm != DialogResult.Yes)
                {
                    return;
                }
            }

            SaveProfile(profileName);
        }

        private string SuggestNewProfileName()
        {
            const string baseName = "New profile";
            var candidate = baseName;
            var suffix = 2;
            while (VoiceSetupStore.ProfileExists(candidate))
            {
                candidate = $"{baseName} {suffix}";
                suffix++;
            }

            return candidate;
        }

        private void SaveProfile(string profileName)
        {
            try
            {
                var profile = CaptureCurrentSetup(profileName);
                VoiceSetupStore.SaveProfile(profile);
                _activeProfileName = profileName;
                RefreshProfileList(profileName);
                pluginStatusLabel.Text = $"Profile saved: {profileName}";
            }
            catch (Exception ex)
            {
                pluginStatusLabel.Text = $"Profile save failed: {ex.Message}";
            }
        }

        private void InitializeTrayIcon()
        {
            _trayMenu = new ContextMenuStrip();
            _trayMenu.Items.Add("Open VST-Applier", null, (_, _) => ShowFromTray());
            _trayMenu.Items.Add(new ToolStripSeparator());
            _trayMenu.Items.Add("Exit", null, (_, _) => ExitApplication());

            _trayIcon = new NotifyIcon
            {
                Icon = LoadApplicationIcon() ?? SystemIcons.Application,
                Text = "VST-Applier",
                ContextMenuStrip = _trayMenu,
                Visible = true,
            };

            _trayIcon.DoubleClick += (_, _) => ShowFromTray();
        }

        private void HideToTray()
        {
            Hide();

            if (_trayIcon is { Visible: true } trayIcon && !_trayBalloonShown)
            {
                _trayBalloonShown = true;
                trayIcon.ShowBalloonTip(
                    3000,
                    "VST-Applier",
                    "Still running in the tray. Double-click the icon to reopen, or right-click it to exit.",
                    ToolTipIcon.Info);
            }
        }

        private void ShowFromTray()
        {
            Show();
            WindowState = FormWindowState.Normal;
            Activate();
            BringToFront();
        }

        /// <summary>
        /// Brings the window to the front. Called by the single-instance listener when
        /// another launch of the executable tries to start a second copy.
        /// </summary>
        internal void RestoreFromTray()
        {
            if (InvokeRequired)
            {
                BeginInvoke(RestoreFromTray);
                return;
            }

            ShowFromTray();
        }

        private void ExitApplication()
        {
            _isExiting = true;
            Close();
        }

        private void settingsButton_Click(object? sender, EventArgs e)
        {
            using var dialog = new SettingsDialog(_appSettings);
            if (dialog.ShowDialog(this) != DialogResult.OK)
            {
                return;
            }

            _appSettings = dialog.Result;
            AppSettingsStore.Save(_appSettings);

            if (_cableSessionGuard is not null)
            {
                _cableSessionGuard.Enabled = _appSettings.KeepCableClean;
            }

            try
            {
                AutoStartManager.SetEnabled(_appSettings.StartWithWindows);
                pluginStatusLabel.Text = "Settings saved";
            }
            catch (Exception ex)
            {
                pluginStatusLabel.Text = $"Settings saved, but Windows startup could not be updated: {ex.Message}";
            }
        }

        protected override void OnFormClosing(FormClosingEventArgs e)
        {
            if (!_isExiting &&
                _appSettings.CloseToTray &&
                e.CloseReason == CloseReason.UserClosing)
            {
                e.Cancel = true;
                HideToTray();
                return;
            }

            base.OnFormClosing(e);
        }

        protected override void OnFormClosed(FormClosedEventArgs e)
        {
            _levelTimer.Stop();

            try
            {
                _audioRoutingService.Stop();
                CloseAllPluginEditors();
                var session = CaptureCurrentSetup(_activeProfileName ?? string.Empty);
                VoiceSetupStore.SaveSession(session);
            }
            catch
            {
                // Session persistence is best effort and must never block shutdown.
            }

            _cableSessionGuard?.Dispose();
            _cableSessionGuard = null;
            _audioRoutingService.Dispose();
            _inputLevelMonitor?.Dispose();
            DisposePluginChain();

            if (_trayIcon is not null)
            {
                _trayIcon.Visible = false;
                _trayIcon.Dispose();
                _trayIcon = null;
            }

            _trayMenu?.Dispose();
            _trayMenu = null;

            base.OnFormClosed(e);
        }

        private void ClosePluginEditor(VstPluginChainItem plugin)
        {
            if (!_pluginEditorForms.TryGetValue(plugin, out var editorForm))
            {
                return;
            }

            if (editorForm.IsDisposed)
            {
                _pluginEditorForms.Remove(plugin);
                return;
            }

            editorForm.Close();
        }

        private void CloseAllPluginEditors()
        {
            foreach (var editorForm in _pluginEditorForms.Values.ToArray())
            {
                if (!editorForm.IsDisposed)
                {
                    editorForm.Close();
                }
            }

            _pluginEditorForms.Clear();
        }

        private void DisposePluginChain()
        {
            foreach (var item in pluginChainListBox.Items)
            {
                if (item is VstPluginChainItem chainItem)
                {
                    chainItem.Dispose();
                }
            }

            pluginChainListBox.Items.Clear();
        }

        private void copyrightLabel_Click(object sender, EventArgs e)
        {
            MessageBox.Show(
                this,
                "VST-Applier v0.1.0\r\n\r\n" +
                "Real-time voice changer that routes your microphone through a VST2/VST3 plugin\r\n" +
                "chain and into a virtual audio cable.\r\n\r\n" +
                "Yousef Mohiey\r\n" +
                "GitHub: github.com/YousefMohiey/VST-Applier\r\n\r\n" +
                "Enjoy your sound!",
                "VST-Applier",
                MessageBoxButtons.OK,
                MessageBoxIcon.Information);
        }
    }
}
