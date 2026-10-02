namespace VstApplier;

using System.ComponentModel;

public sealed class AudioLevelMeterControl : Control
{
    private float _level;
    private float _peakPosition;
    private long _lastPeakUpdateMs;
    private long _peakHoldUntilMs;

    public AudioLevelMeterControl()
    {
        SetStyle(
            ControlStyles.AllPaintingInWmPaint |
            ControlStyles.OptimizedDoubleBuffer |
            ControlStyles.ResizeRedraw |
            ControlStyles.UserPaint,
            true);
    }

    [Browsable(false)]
    [DesignerSerializationVisibility(DesignerSerializationVisibility.Hidden)]
    public float Level
    {
        get => _level;
        set
        {
            var nextLevel = Math.Clamp(value, 0, 1);
            var levelChanged = Math.Abs(nextLevel - _level) >= 0.002f;
            var peakChanged = UpdatePeakHold(nextLevel);

            if (!levelChanged && !peakChanged)
            {
                return;
            }

            _level = nextLevel;
            Invalidate();
        }
    }

    protected override void OnPaint(PaintEventArgs e)
    {
        base.OnPaint(e);

        var graphics = e.Graphics;
        graphics.Clear(Color.FromArgb(4, 24, 33));

        var meterBounds = new Rectangle(16, 12, Math.Max(12, Width - 32), Math.Max(16, Height - 30));
        using var borderPen = new Pen(Color.FromArgb(33, 104, 120));
        graphics.DrawRectangle(borderPen, meterBounds);

        var displayLevel = ToMeterPosition(_level);
        var litHeight = (int)(meterBounds.Height * displayLevel);
        var litTop = meterBounds.Bottom - litHeight;

        DrawScale(graphics, meterBounds);

        if (litHeight > 0)
        {
            using var greenBrush = new SolidBrush(Color.FromArgb(0, 255, 196));
            graphics.FillRectangle(greenBrush, meterBounds.Left + 1, litTop, meterBounds.Width - 1, litHeight);

            if (displayLevel > 0.72f)
            {
                var hotHeight = (int)(meterBounds.Height * (displayLevel - 0.72f));
                using var yellowBrush = new SolidBrush(Color.FromArgb(0, 180, 181));
                graphics.FillRectangle(yellowBrush, meterBounds.Left + 1, meterBounds.Bottom - litHeight, meterBounds.Width - 1, hotHeight);
            }

            if (displayLevel > 0.9f)
            {
                var clipHeight = (int)(meterBounds.Height * (displayLevel - 0.9f));
                using var redBrush = new SolidBrush(Color.FromArgb(255, 77, 123));
                graphics.FillRectangle(redBrush, meterBounds.Left + 1, meterBounds.Bottom - litHeight, meterBounds.Width - 1, clipHeight);
            }
        }

        var peakY = meterBounds.Bottom - (int)(meterBounds.Height * _peakPosition);
        using var peakPen = new Pen(Color.FromArgb(231, 246, 244), 2);
        graphics.DrawLine(peakPen, meterBounds.Left + 1, peakY, meterBounds.Right - 1, peakY);

        using var captionBrush = new SolidBrush(Color.FromArgb(0, 255, 196));
        graphics.DrawString("RMS", Font, captionBrush, 6, Height - 18);
        graphics.DrawString(ToDecibels(_level), Font, captionBrush, Math.Max(34, Width - 52), Height - 18);
    }

    private static void DrawScale(Graphics graphics, Rectangle meterBounds)
    {
        using var linePen = new Pen(Color.FromArgb(21, 76, 90));

        foreach (var value in new[] { 0.25f, 0.5f, 0.75f })
        {
            var y = meterBounds.Bottom - (int)(meterBounds.Height * value);
            graphics.DrawLine(linePen, meterBounds.Left + 1, y, meterBounds.Right - 1, y);
        }
    }

    private static string ToDecibels(float level)
    {
        if (level <= 0.0001f)
        {
            return "-inf";
        }

        var decibels = 20 * Math.Log10(level);
        return $"{decibels:0.0}";
    }

    private static float ToMeterPosition(float level)
    {
        const float minDecibels = -60f;

        if (level <= 0.0001f)
        {
            return 0;
        }

        var decibels = 20f * (float)Math.Log10(level);
        return Math.Clamp((decibels - minDecibels) / -minDecibels, 0, 1);
    }

    private bool UpdatePeakHold(float level)
    {
        const int holdMilliseconds = 1000;
        const float fallPerSecond = 0.38f;

        var nowMs = Environment.TickCount64;
        if (_lastPeakUpdateMs == 0)
        {
            _lastPeakUpdateMs = nowMs;
        }

        var elapsedSeconds = Math.Max(0, nowMs - _lastPeakUpdateMs) / 1000f;
        _lastPeakUpdateMs = nowMs;

        var previousPeakPosition = _peakPosition;
        var displayLevel = ToMeterPosition(level);

        if (displayLevel >= _peakPosition)
        {
            _peakPosition = displayLevel;
            _peakHoldUntilMs = nowMs + holdMilliseconds;
        }
        else if (nowMs > _peakHoldUntilMs)
        {
            _peakPosition = Math.Max(displayLevel, _peakPosition - fallPerSecond * elapsedSeconds);
        }

        return Math.Abs(previousPeakPosition - _peakPosition) >= 0.002f;
    }
}
