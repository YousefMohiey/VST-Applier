using System.Drawing.Drawing2D;

namespace VstApplier;

public sealed class DarkComboBox : ComboBox
{
    private Color _background = Color.FromArgb(4, 24, 33);
    private Color _foreground = Color.FromArgb(231, 246, 244);
    private Color _selectedBackground = Color.FromArgb(0, 255, 196);
    private Color _selectedForeground = Color.FromArgb(5, 32, 43);
    private Color _border = Color.FromArgb(33, 104, 120);
    private Color _disabledForeground = Color.FromArgb(128, 176, 182);
    private int _cornerRadius = 5;

    public DarkComboBox()
    {
        BackColor = _background;
        ForeColor = _foreground;
        DrawMode = DrawMode.OwnerDrawFixed;
        DropDownStyle = ComboBoxStyle.DropDownList;
        FlatStyle = FlatStyle.Flat;
    }

    public void ApplyTheme(
        Color background,
        Color foreground,
        Color selectedBackground,
        Color selectedForeground,
        Color border,
        Color disabledForeground,
        int cornerRadius)
    {
        _background = background;
        _foreground = foreground;
        _selectedBackground = selectedBackground;
        _selectedForeground = selectedForeground;
        _border = border;
        _disabledForeground = disabledForeground;
        _cornerRadius = cornerRadius;

        BackColor = _background;
        ForeColor = _foreground;
        DrawMode = DrawMode.OwnerDrawFixed;
        FlatStyle = FlatStyle.Flat;
        Invalidate();
    }

    protected override void OnDrawItem(DrawItemEventArgs e)
    {
        if (e.Bounds.Width <= 0 || e.Bounds.Height <= 0)
        {
            return;
        }

        var isSelected = (e.State & DrawItemState.Selected) == DrawItemState.Selected;
        var isDisabled = (e.State & DrawItemState.Disabled) == DrawItemState.Disabled || !Enabled;
        var background = isSelected && !isDisabled
            ? _selectedBackground
            : _background;
        var foreground = isDisabled
            ? _disabledForeground
            : isSelected ? _selectedForeground : _foreground;

        using var backgroundBrush = new SolidBrush(background);
        e.Graphics.FillRectangle(backgroundBrush, e.Bounds);

        var text = e.Index >= 0 && e.Index < Items.Count
            ? GetItemText(Items[e.Index])
            : Text;
        var textBounds = new Rectangle(
            e.Bounds.Left + 4,
            e.Bounds.Top,
            Math.Max(0, e.Bounds.Width - 8),
            e.Bounds.Height);

        TextRenderer.DrawText(
            e.Graphics,
            text,
            Font,
            textBounds,
            foreground,
            TextFormatFlags.Left |
            TextFormatFlags.VerticalCenter |
            TextFormatFlags.EndEllipsis |
            TextFormatFlags.NoPrefix);
    }

    protected override void OnEnabledChanged(EventArgs e)
    {
        base.OnEnabledChanged(e);
        Invalidate();
    }

    protected override void OnSelectedIndexChanged(EventArgs e)
    {
        base.OnSelectedIndexChanged(e);
        Invalidate();
    }

    protected override void WndProc(ref Message m)
    {
        base.WndProc(ref m);

        if (m.Msg is 0x000F or 0x0085)
        {
            using var graphics = Graphics.FromHwnd(Handle);
            DrawChrome(graphics);
        }
    }

    private void DrawChrome(Graphics graphics)
    {
        if (Width <= 0 || Height <= 0)
        {
            return;
        }

        graphics.SmoothingMode = SmoothingMode.AntiAlias;

        var arrowWidth = Math.Max(20, SystemInformation.HorizontalScrollBarArrowWidth);
        var arrowBounds = new Rectangle(
            Math.Max(0, Width - arrowWidth - 1),
            1,
            arrowWidth,
            Math.Max(0, Height - 2));

        using var arrowBackgroundBrush = new SolidBrush(_background);
        graphics.FillRectangle(arrowBackgroundBrush, arrowBounds);

        var arrowColor = Enabled ? _foreground : _disabledForeground;
        using var arrowPen = new Pen(arrowColor, 1.5f)
        {
            StartCap = LineCap.Round,
            EndCap = LineCap.Round,
            LineJoin = LineJoin.Round
        };

        var centerX = arrowBounds.Left + arrowBounds.Width / 2;
        var centerY = arrowBounds.Top + arrowBounds.Height / 2;
        var arrowSize = 4;
        graphics.DrawLines(
            arrowPen,
            new[]
            {
                new Point(centerX - arrowSize, centerY - 2),
                new Point(centerX, centerY + 2),
                new Point(centerX + arrowSize, centerY - 2)
            });

        var borderBounds = new Rectangle(0, 0, Width - 1, Height - 1);
        using var borderPath = CreateRoundedRectanglePath(borderBounds, _cornerRadius);
        using var borderPen = new Pen(_border);
        graphics.DrawPath(borderPen, borderPath);
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
}
