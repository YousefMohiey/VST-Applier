using System.Drawing.Drawing2D;

namespace VstApplier;

public sealed class SettingsDialog : Form
{
    private static readonly Color DialogBackground = Color.FromArgb(5, 32, 43);
    private static readonly Color SurfaceBackground = Color.FromArgb(10, 47, 60);
    private static readonly Color ButtonBackground = Color.FromArgb(14, 57, 71);
    private static readonly Color ButtonHover = Color.FromArgb(21, 76, 92);
    private static readonly Color ButtonPressed = Color.FromArgb(27, 92, 110);
    private static readonly Color PrimaryText = Color.FromArgb(231, 246, 244);
    private static readonly Color SecondaryText = Color.FromArgb(128, 176, 182);
    private static readonly Color AccentMint = Color.FromArgb(0, 255, 196);
    private static readonly Color OnAccent = Color.FromArgb(5, 32, 43);
    private static readonly Color BorderLine = Color.FromArgb(21, 76, 90);
    private static readonly Color SoftBorder = Color.FromArgb(33, 104, 120);

    private readonly CheckBox _autoStartMicrophoneCheckBox;
    private readonly CheckBox _closeToTrayCheckBox;
    private readonly CheckBox _startWithWindowsCheckBox;
    private readonly CheckBox _keepCableCleanCheckBox;

    public SettingsDialog(AppSettings settings)
    {
        Text = "Settings";
        FormBorderStyle = FormBorderStyle.FixedDialog;
        StartPosition = FormStartPosition.CenterParent;
        MaximizeBox = false;
        MinimizeBox = false;
        ShowInTaskbar = false;
        ClientSize = new Size(430, 246);
        BackColor = DialogBackground;
        ForeColor = PrimaryText;
        AutoScaleMode = AutoScaleMode.None;

        var header = new Label
        {
            Text = "Startup and tray behavior",
            ForeColor = SecondaryText,
            AutoSize = true,
            Location = new Point(16, 14),
        };

        _autoStartMicrophoneCheckBox = CreateCheckBox(
            "Start microphone routing automatically when the app opens",
            settings.AutoStartMicrophone,
            new Point(16, 44));

        _closeToTrayCheckBox = CreateCheckBox(
            "Keep running in the system tray when the window is closed",
            settings.CloseToTray,
            new Point(16, 76));

        _startWithWindowsCheckBox = CreateCheckBox(
            "Start with Windows (opens minimized to tray)",
            settings.StartWithWindows,
            new Point(16, 108));

        _keepCableCleanCheckBox = CreateCheckBox(
            "Mute other apps on the virtual cable",
            settings.KeepCableClean,
            new Point(16, 140));

        var hint = new Label
        {
            Text = "Use the tray icon to reopen the window or exit completely.",
            ForeColor = SecondaryText,
            AutoSize = true,
            Location = new Point(16, 172),
        };

        var saveButton = CreateDialogButton("Save", new Point(252, 204), DialogResult.OK, isPrimary: true);
        var cancelButton = CreateDialogButton("Cancel", new Point(334, 204), DialogResult.Cancel);

        Controls.Add(header);
        Controls.Add(_autoStartMicrophoneCheckBox);
        Controls.Add(_closeToTrayCheckBox);
        Controls.Add(_startWithWindowsCheckBox);
        Controls.Add(_keepCableCleanCheckBox);
        Controls.Add(hint);
        Controls.Add(saveButton);
        Controls.Add(cancelButton);

        AcceptButton = saveButton;
        CancelButton = cancelButton;
    }

    public AppSettings Result => new()
    {
        AutoStartMicrophone = _autoStartMicrophoneCheckBox.Checked,
        CloseToTray = _closeToTrayCheckBox.Checked,
        StartWithWindows = _startWithWindowsCheckBox.Checked,
        KeepCableClean = _keepCableCleanCheckBox.Checked,
    };

    private static CheckBox CreateCheckBox(string text, bool isChecked, Point location)
    {
        return new CheckBox
        {
            Text = text,
            Checked = isChecked,
            AutoSize = true,
            FlatStyle = FlatStyle.Flat,
            BackColor = SurfaceBackground,
            ForeColor = PrimaryText,
            Location = location,
        };
    }

    protected override void OnHandleCreated(EventArgs e)
    {
        base.OnHandleCreated(e);
        DarkTitleBar.Apply(this);
    }

    private static Button CreateDialogButton(string text, Point location, DialogResult dialogResult, bool isPrimary = false)
    {
        var button = new Button
        {
            Text = text,
            Location = location,
            Size = new Size(78, 28),
            FlatStyle = FlatStyle.Flat,
            BackColor = ButtonBackground,
            ForeColor = PrimaryText,
            UseVisualStyleBackColor = false,
            DialogResult = dialogResult,
        };

        button.FlatAppearance.BorderSize = 0;
        button.FlatAppearance.BorderColor = SoftBorder;
        button.FlatAppearance.MouseOverBackColor = ButtonHover;
        button.FlatAppearance.MouseDownBackColor = ButtonPressed;
        button.Paint += (_, e) => DrawButton(button, isPrimary, e);
        button.MouseEnter += (_, _) => button.Invalidate();
        button.MouseLeave += (_, _) => button.Invalidate();
        button.MouseDown += (_, _) => button.Invalidate();
        button.MouseUp += (_, _) => button.Invalidate();

        return button;
    }

    private static void DrawButton(Button button, bool isPrimary, PaintEventArgs e)
    {
        var hovered = button.Enabled &&
            button.ClientRectangle.Contains(button.PointToClient(Cursor.Position));
        var pressed = hovered && (Control.MouseButtons & MouseButtons.Left) == MouseButtons.Left;

        var background = isPrimary
            ? pressed
                ? Color.FromArgb(0, 214, 164)
                : hovered ? Color.FromArgb(64, 255, 210) : AccentMint
            : pressed
                ? ButtonPressed
                : hovered ? ButtonHover : ButtonBackground;
        var foreground = isPrimary ? OnAccent : PrimaryText;
        var border = isPrimary ? background : hovered ? SoftBorder : BorderLine;

        e.Graphics.SmoothingMode = SmoothingMode.AntiAlias;
        e.Graphics.Clear(button.Parent?.BackColor ?? DialogBackground);

        var borderBounds = new Rectangle(0, 0, button.Width - 1, button.Height - 1);
        using var path = new GraphicsPath();
        const int radius = 5;
        var diameter = radius * 2;
        path.AddArc(borderBounds.Left, borderBounds.Top, diameter, diameter, 180, 90);
        path.AddArc(borderBounds.Right - diameter, borderBounds.Top, diameter, diameter, 270, 90);
        path.AddArc(borderBounds.Right - diameter, borderBounds.Bottom - diameter, diameter, diameter, 0, 90);
        path.AddArc(borderBounds.Left, borderBounds.Bottom - diameter, diameter, diameter, 90, 90);
        path.CloseFigure();

        using var backgroundBrush = new SolidBrush(background);
        e.Graphics.FillPath(backgroundBrush, path);

        using var borderPen = new Pen(border);
        e.Graphics.DrawPath(borderPen, path);

        TextRenderer.DrawText(
            e.Graphics,
            button.Text,
            button.Font,
            button.ClientRectangle,
            foreground,
            TextFormatFlags.HorizontalCenter |
            TextFormatFlags.VerticalCenter |
            TextFormatFlags.NoPrefix);
    }
}
