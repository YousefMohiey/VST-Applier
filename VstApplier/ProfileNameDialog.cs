using System.Drawing.Drawing2D;

namespace VstApplier;

public sealed class ProfileNameDialog : Form
{
    private static readonly Color DialogBackground = Color.FromArgb(5, 32, 43);
    private static readonly Color ControlBackground = Color.FromArgb(4, 24, 33);
    private static readonly Color ButtonBackground = Color.FromArgb(14, 57, 71);
    private static readonly Color ButtonHover = Color.FromArgb(21, 76, 92);
    private static readonly Color ButtonPressed = Color.FromArgb(27, 92, 110);
    private static readonly Color PrimaryText = Color.FromArgb(231, 246, 244);
    private static readonly Color SecondaryText = Color.FromArgb(128, 176, 182);
    private static readonly Color DangerText = Color.FromArgb(255, 77, 123);
    private static readonly Color AccentMint = Color.FromArgb(0, 255, 196);
    private static readonly Color OnAccent = Color.FromArgb(5, 32, 43);
    private static readonly Color BorderLine = Color.FromArgb(21, 76, 90);
    private static readonly Color SoftBorder = Color.FromArgb(33, 104, 120);

    private readonly TextBox _nameTextBox;
    private readonly Label _errorLabel;

    public ProfileNameDialog(string suggestedName)
    {
        Text = "Create profile";
        FormBorderStyle = FormBorderStyle.FixedDialog;
        StartPosition = FormStartPosition.CenterParent;
        MaximizeBox = false;
        MinimizeBox = false;
        ShowInTaskbar = false;
        ClientSize = new Size(360, 138);
        BackColor = DialogBackground;
        ForeColor = PrimaryText;
        AutoScaleMode = AutoScaleMode.None;

        var nameLabel = new Label
        {
            Text = "Profile name",
            ForeColor = SecondaryText,
            AutoSize = true,
            Location = new Point(16, 16),
        };

        _nameTextBox = new TextBox
        {
            Text = suggestedName,
            BackColor = ControlBackground,
            ForeColor = PrimaryText,
            BorderStyle = BorderStyle.FixedSingle,
            Location = new Point(16, 40),
            Size = new Size(328, 27),
        };

        _errorLabel = new Label
        {
            Text = string.Empty,
            ForeColor = DangerText,
            AutoEllipsis = true,
            Location = new Point(16, 72),
            Size = new Size(328, 20),
        };

        var okButton = CreateDialogButton("Create", new Point(182, 98), DialogResult.OK, isPrimary: true);
        okButton.Click += OkButton_Click;

        var cancelButton = CreateDialogButton("Cancel", new Point(264, 98), DialogResult.Cancel);

        Controls.Add(nameLabel);
        Controls.Add(_nameTextBox);
        Controls.Add(_errorLabel);
        Controls.Add(okButton);
        Controls.Add(cancelButton);

        AcceptButton = okButton;
        CancelButton = cancelButton;
    }

    public string ProfileName => _nameTextBox.Text.Trim();

    private void OkButton_Click(object? sender, EventArgs e)
    {
        if (!VoiceSetupStore.IsValidProfileName(_nameTextBox.Text, out var error))
        {
            _errorLabel.Text = error;
            DialogResult = DialogResult.None;
            return;
        }

        DialogResult = DialogResult.OK;
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
