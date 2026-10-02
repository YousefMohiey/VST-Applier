namespace VstApplier;

using NAudio.CoreAudioApi;
using NAudio.Wave;

public sealed class AudioInputLevelMonitor : IDisposable
{
    private readonly object _levelLock = new();
    private MMDeviceEnumerator? _deviceEnumerator;
    private MMDevice? _device;
    private WasapiCapture? _capture;
    private float _peakLevel;

    public AudioInputLevelMonitor(AudioInputDevice inputDevice)
    {
        Device = inputDevice;
        _deviceEnumerator = new MMDeviceEnumerator();
        _device = _deviceEnumerator.GetDevice(inputDevice.Id);
        _capture = new WasapiCapture(_device);
        _capture.DataAvailable += Capture_DataAvailable;
        _capture.StartRecording();
    }

    public AudioInputDevice Device { get; }

    public float GetPeakLevel()
    {
        lock (_levelLock)
        {
            return _peakLevel;
        }
    }

    private void Capture_DataAvailable(object? sender, WaveInEventArgs e)
    {
        if (_capture is null || e.BytesRecorded <= 0)
        {
            SetPeakLevel(0);
            return;
        }

        SetPeakLevel(AudioBufferLevelCalculator.CalculatePeakLevel(
            e.Buffer,
            e.BytesRecorded,
            _capture.WaveFormat));
    }

    private void SetPeakLevel(float level)
    {
        lock (_levelLock)
        {
            _peakLevel = Math.Clamp(level, 0, 1);
        }
    }

    public void Dispose()
    {
        if (_capture is not null)
        {
            _capture.DataAvailable -= Capture_DataAvailable;
            _capture.StopRecording();
            _capture.Dispose();
        }

        _device?.Dispose();
        _deviceEnumerator?.Dispose();
        _capture = null;
        _device = null;
        _deviceEnumerator = null;
    }
}
