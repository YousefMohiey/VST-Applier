using System.Drawing.Drawing2D;

namespace VstApplier;

public sealed class ProfileNameDialog : Form
{
    private static readonly Color DialogBackground = Color.FromArgb(18, 18, 18);
    private static readonly Color ControlBackground = Color.FromArgb(24, 24, 24);
    private static readonly Color ButtonBackground = Color.FromArgb(43, 43, 43);
    private static readonly Color PrimaryText = Color.FromArgb(234, 234, 234);
    private static readonly Color SecondaryText = Color.FromArgb(166, 166, 166);
    private static readonly Color DangerText = Color.FromArgb(255, 107, 107);

    private readonly TextBox _nameTextBox;
    private readonly Label _errorLabel;

    public ProfileNameDialog(string suggestedName)
    {
        Text = "Save profile";
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

        var okButton = CreateDialogButton("Save", new Point(182, 98), DialogResult.OK);
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
