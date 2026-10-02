using System.Drawing.Drawing2D;

namespace VstApplier;

public sealed class SettingsDialog : Form
{
    private static readonly Color DialogBackground = Color.FromArgb(18, 18, 18);
    private static readonly Color SurfaceBackground = Color.FromArgb(30, 30, 30);
    private static readonly Color ButtonBackground = Color.FromArgb(43, 43, 43);
    private static readonly Color PrimaryText = Color.FromArgb(234, 234, 234);
    private static readonly Color SecondaryText = Color.FromArgb(166, 166, 166);

    private readonly CheckBox _autoStartMicrophoneCheckBox;
    private readonly CheckBox _closeToTrayCheckBox;
    private readonly CheckBox _startWithWindowsCheckBox;

    public SettingsDialog(AppSettings settings)
    {
        Text = "Settings";
        FormBorderStyle = FormBorderStyle.FixedDialog;
        StartPosition = FormStartPosition.CenterParent;
        MaximizeBox = false;
        MinimizeBox = false;
        ShowInTaskbar = false;
        ClientSize = new Size(430, 214);
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

        var hint = new Label
        {
            Text = "Use the tray icon to reopen the window or exit completely.",
            ForeColor = SecondaryText,
            AutoSize = true,
            Location = new Point(16, 140),
        };

        var saveButton = CreateDialogButton("Save", new Point(252, 172), DialogResult.OK);
        var cancelButton = CreateDialogButton("Cancel", new Point(334, 172), DialogResult.Cancel);

        Controls.Add(header);
        Controls.Add(_autoStartMicrophoneCheckBox);
        Controls.Add(_closeToTrayCheckBox);
        Controls.Add(_startWithWindowsCheckBox);
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

    private static Button CreateDialogButton(string text, Point location, DialogResult dialogResult)
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
        button.FlatAppearance.BorderColor = Color.FromArgb(112, 112, 112);
        button.FlatAppearance.MouseOverBackColor = Color.FromArgb(55, 55, 55);
        button.FlatAppearance.MouseDownBackColor = Color.FromArgb(65, 65, 65);
        button.Paint += (_, e) => DrawButton(button, e);

        return button;
    }

    private static void DrawButton(Button button, PaintEventArgs e)
    {
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

        using var backgroundBrush = new SolidBrush(ButtonBackground);
        e.Graphics.FillPath(backgroundBrush, path);

        using var borderPen = new Pen(Color.FromArgb(112, 112, 112));
        e.Graphics.DrawPath(borderPen, path);

        TextRenderer.DrawText(
            e.Graphics,
            button.Text,
            button.Font,
            button.ClientRectangle,
            PrimaryText,
            TextFormatFlags.HorizontalCenter |
            TextFormatFlags.VerticalCenter |
            TextFormatFlags.NoPrefix);
    }
}
