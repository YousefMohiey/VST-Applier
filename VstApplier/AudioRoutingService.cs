namespace VstApplier;

using System.Diagnostics;
using System.Runtime;
using System.Runtime.InteropServices;
using NAudio.CoreAudioApi;
using NAudio.Wave;

public sealed class AudioRoutingService : IDisposable
{
    private const double InitialPreloadSeconds = 0.10;

    // Latency budget: when the CPU is busy, capture bursts can push a backlog into the
    // route buffer, and that extra delay would stay there (permanent lag). Instead the
    // oldest audio is dropped, keeping the delay bounded (a short blip beats lag).
    private const double MaxBufferedMilliseconds = 300;
    private const double TargetBufferedMilliseconds = 150;

    private const int AvrtPriorityHigh = 1;

    private readonly object _levelLock = new();
    private readonly object _pluginChainLock = new();
    private readonly object _processingLock = new();
    private MMDeviceEnumerator? _deviceEnumerator;
    private MMDevice? _inputDevice;
    private MMDevice? _outputDevice;
    private WasapiCapture? _capture;
    private WasapiOut? _output;
    private AudioRouteBuffer? _routeBuffer;
    private VstPluginChainItem[] _activePluginChain = Array.Empty<VstPluginChainItem>();
    private int _maxPluginBlockSize;
    private int _pluginProcessingChannels;
    private string _processingStatus = "VST bypassed";
    private float _inputPeakLevel;
    private float _outputPeakLevel;
    private double _resamplePosition = 1.0;
    private float _resampleLastSample;
    private bool _resamplerHasLastSample;
    private bool _outputStarted;
    private int _requestedOutputLatencyMs;
    private double _lastCaptureBlockMs;
    private bool _audioThreadPriorityApplied;
    private GCLatencyMode _previousLatencyMode = GCLatencyMode.Interactive;
    private ProcessPriorityClass _previousProcessPriority = ProcessPriorityClass.Normal;

    // Reused processing buffers. The capture callback runs at audio rate, so the hot
    // path must not allocate; allocations trigger GC pauses that glitch the audio.
    private float[] _captureInputBuffer = Array.Empty<float>();
    private float[] _monoBuffer = Array.Empty<float>();
    private float[] _pluginBufferA = Array.Empty<float>();
    private float[] _pluginBufferB = Array.Empty<float>();
    private float[] _resampleBuffer = Array.Empty<float>();
    private float[] _routeOutputBuffer = Array.Empty<float>();

    public bool IsRunning => _capture is not null && _output is not null;

    public bool IsVstProcessingActive
    {
        get
        {
            lock (_pluginChainLock)
            {
                return _activePluginChain.Length > 0;
            }
        }
    }

    public string ProcessingStatus
    {
        get
        {
            lock (_pluginChainLock)
            {
                return _processingStatus;
            }
        }
    }

    public void Start(
        AudioInputDevice inputDevice,
        AudioOutputDevice outputDevice,
        IReadOnlyList<VstPluginChainItem>? pluginChain = null,
        int pluginBlockSize = 512)
    {
        Stop();

        _audioThreadPriorityApplied = false;
        try
        {
            _previousLatencyMode = GCSettings.LatencyMode;
            GCSettings.LatencyMode = GCLatencyMode.SustainedLowLatency;
        }
        catch
        {
            // GC latency mode is best effort.
        }

        try
        {
            // A mild process boost keeps the whole audio pipeline (capture, plugin
            // processing, output) scheduled ahead of background work while the CPU is
            // busy; games run at High and are not affected. Restored on stop.
            _previousProcessPriority = Process.GetCurrentProcess().PriorityClass;
            Process.GetCurrentProcess().PriorityClass = ProcessPriorityClass.AboveNormal;
        }
        catch
        {
            // Process priority is best effort.
        }

        _deviceEnumerator = new MMDeviceEnumerator();
        _inputDevice = _deviceEnumerator.GetDevice(inputDevice.Id);
        _outputDevice = _deviceEnumerator.GetDevice(outputDevice.Id);

        _capture = new WasapiCapture(_inputDevice);
        var routeLatency = CalculateRouteLatency(_capture.WaveFormat, pluginBlockSize);
        _requestedOutputLatencyMs = routeLatency.OutputLatencyMs;
        _lastCaptureBlockMs = 0;
        var outputFormat = _outputDevice.AudioClient.MixFormat;
        var routeFormat = CreateRouteWaveFormat(outputFormat);
        _routeBuffer = new AudioRouteBuffer(routeFormat, TimeSpan.FromMilliseconds(1000));
        ResetResamplerState();

        ConfigurePluginChain(pluginChain ?? Array.Empty<VstPluginChainItem>(), _capture.WaveFormat, pluginBlockSize);

        _output = new WasapiOut(_outputDevice, AudioClientShareMode.Shared, true, routeLatency.OutputLatencyMs);
        _output.Init(_routeBuffer);
        _capture.DataAvailable += Capture_DataAvailable;

        _capture.StartRecording();
    }

    public AudioRouteDiagnostics GetDiagnostics()
    {
        lock (_processingLock)
        {
            return new AudioRouteDiagnostics(
                _routeBuffer?.BufferedMilliseconds ?? 0,
                _routeBuffer?.CapacityMilliseconds ?? 0,
                _requestedOutputLatencyMs,
                _lastCaptureBlockMs,
                _maxPluginBlockSize,
                InitialPreloadSeconds * 1000);
        }
    }

    public float GetOutputPeakLevel()
    {
        lock (_levelLock)
        {
            return _outputPeakLevel;
        }
    }

    public float GetInputPeakLevel()
    {
        lock (_levelLock)
        {
            return _inputPeakLevel;
        }
    }

    public void Stop()
    {
        if (_capture is not null)
        {
            _capture.DataAvailable -= Capture_DataAvailable;
            _capture.StopRecording();
            _capture.Dispose();
        }

        lock (_processingLock)
        {
            _output?.Stop();
            _output?.Dispose();
            _inputDevice?.Dispose();
            _outputDevice?.Dispose();
            _deviceEnumerator?.Dispose();

            _capture = null;
            _output = null;
            _routeBuffer = null;
            _inputDevice = null;
            _outputDevice = null;
            _deviceEnumerator = null;
            _outputStarted = false;
            _requestedOutputLatencyMs = 0;
            _lastCaptureBlockMs = 0;
            _audioThreadPriorityApplied = false;
            ResetResamplerState();
            ClearPluginProcessing("VST bypassed");
            SetInputPeakLevel(0);
            SetOutputPeakLevel(0);
        }

        try
        {
            GCSettings.LatencyMode = _previousLatencyMode;
        }
        catch
        {
            // GC latency mode is best effort.
        }

        try
        {
            Process.GetCurrentProcess().PriorityClass = _previousProcessPriority;
        }
        catch
        {
            // Process priority is best effort.
        }
    }

    private void Capture_DataAvailable(object? sender, WaveInEventArgs e)
    {
        EnsureAudioThreadPriority();

        lock (_processingLock)
        {
            CaptureDataAvailableCore(e);
        }
    }

    private void CaptureDataAvailableCore(WaveInEventArgs e)
    {
        if (_capture is null || _routeBuffer is null || _output is null || e.BytesRecorded <= 0)
        {
            SetOutputPeakLevel(0);
            return;
        }

        _lastCaptureBlockMs = CalculateCaptureBlockMilliseconds(e.BytesRecorded, _capture.WaveFormat);

        var routeSampleCount = ProcessCaptureBlockToRouteSamples(
            e.Buffer,
            e.BytesRecorded,
            _capture.WaveFormat,
            _routeBuffer.WaveFormat);

        SetOutputPeakLevel(CalculatePeakLevel(_routeOutputBuffer.AsSpan(0, routeSampleCount)));
        _routeBuffer.AddSamples(_routeOutputBuffer.AsSpan(0, routeSampleCount));
        EnforceLatencyBudget(_routeBuffer);

        if (!_outputStarted && HasEnoughSamplesToStart(_routeBuffer))
        {
            _output.Play();
            _outputStarted = true;
        }
    }

    /// <summary>
    /// Runs on the audio processing thread (the capture callback thread) on its first
    /// call. Raises the thread priority and registers it with the Windows multimedia
    /// scheduler ("Pro Audio", the mechanism DAWs use), so the audio keeps its time
    /// slices while the CPU is busy. The registration ends with the thread.
    /// </summary>
    private void EnsureAudioThreadPriority()
    {
        if (_audioThreadPriorityApplied)
        {
            return;
        }

        _audioThreadPriorityApplied = true;

        try
        {
            Thread.CurrentThread.Priority = ThreadPriority.Highest;
        }
        catch
        {
            // Thread priority is best effort.
        }

        try
        {
            uint taskIndex = 0;
            var mmcssHandle = AvSetMmThreadCharacteristicsW("Pro Audio", ref taskIndex);
            if (mmcssHandle != IntPtr.Zero)
            {
                AvSetMmThreadPriority(mmcssHandle, AvrtPriorityHigh);
            }
        }
        catch
        {
            // Falls back to the raised thread priority above.
        }
    }

    /// <summary>
    /// Drops the oldest buffered audio when the backlog exceeds the budget, so a CPU
    /// spike cannot leave a permanent delay in the route. A short blip beats lag.
    /// </summary>
    private static void EnforceLatencyBudget(AudioRouteBuffer routeBuffer)
    {
        if (routeBuffer.BufferedMilliseconds <= MaxBufferedMilliseconds)
        {
            return;
        }

        routeBuffer.TrimOldestMilliseconds(TargetBufferedMilliseconds);
    }

    private void ConfigurePluginChain(
        IReadOnlyList<VstPluginChainItem> pluginChain,
        WaveFormat captureFormat,
        int pluginBlockSize)
    {
        pluginBlockSize = NormalizePluginBlockSize(pluginBlockSize);

        if (pluginChain.Count == 0)
        {
            ClearPluginProcessing($"VST bypassed: empty chain, block {pluginBlockSize}");
            return;
        }

        if (!AudioSampleConverter.IsSupported(captureFormat))
        {
            ClearPluginProcessing($"VST bypassed: unsupported {captureFormat.BitsPerSample}-bit {captureFormat.Encoding} capture format");
            return;
        }

        var channelCount = captureFormat.Channels;
        if (channelCount <= 0)
        {
            ClearPluginProcessing($"VST bypassed: unsupported {channelCount}-channel capture format");
            return;
        }

        var maxBlockSize = pluginBlockSize;
        var pluginChannelCount = GetPluginProcessingChannelCount();
        var setupChain = pluginChain.ToArray();

        try
        {
            foreach (var plugin in setupChain)
            {
                plugin.SetupProcessing(
                    captureFormat.SampleRate,
                    maxBlockSize,
                    pluginChannelCount,
                    pluginChannelCount);
            }
        }
        catch (Exception ex)
        {
            ClearPluginProcessing($"VST bypassed: setup failed ({ex.Message})");
            return;
        }

        lock (_pluginChainLock)
        {
            _activePluginChain = setupChain;
            _maxPluginBlockSize = maxBlockSize;
            _pluginProcessingChannels = pluginChannelCount;
            _processingStatus = $"VST active: {setupChain.Length} plugin(s), block {maxBlockSize}, capture {channelCount}ch -> VST {pluginChannelCount}ch";
        }
    }

    private int ProcessCaptureBlockToRouteSamples(
        byte[] sourceBuffer,
        int byteCount,
        WaveFormat captureFormat,
        WaveFormat routeFormat)
    {
        var frameCount = AudioSampleConverter.GetFrameCount(byteCount, captureFormat);
        var channelCount = captureFormat.Channels;
        var captureSampleCount = frameCount * channelCount;

        _captureInputBuffer = EnsureBuffer(_captureInputBuffer, captureSampleCount);
        AudioSampleConverter.ConvertToFloat32(
            sourceBuffer.AsSpan(0, byteCount),
            captureFormat,
            _captureInputBuffer);

        _monoBuffer = EnsureBuffer(_monoBuffer, frameCount);
        DownmixToMonoInto(_captureInputBuffer, frameCount, channelCount, _monoBuffer);
        SetInputPeakLevel(CalculatePeakLevel(_monoBuffer.AsSpan(0, frameCount)));

        var pluginChain = GetActivePluginChain();
        if (pluginChain.Length > 0)
        {
            try
            {
                ProcessMonoWithPluginChain(pluginChain, frameCount);
            }
            catch (Exception ex)
            {
                ClearPluginProcessing($"VST bypassed: {ex.Message}");
            }
        }

        return ConvertMonoToRouteSamples(frameCount, captureFormat.SampleRate, routeFormat);
    }

    private void ProcessMonoWithPluginChain(
        IReadOnlyList<VstPluginChainItem> pluginChain,
        int frameCount)
    {
        var pluginChannelCount = _pluginProcessingChannels <= 0
            ? GetPluginProcessingChannelCount()
            : _pluginProcessingChannels;
        var pluginSampleCount = frameCount * pluginChannelCount;

        _pluginBufferA = EnsureBuffer(_pluginBufferA, pluginSampleCount);
        _pluginBufferB = EnsureBuffer(_pluginBufferB, pluginSampleCount);

        var current = _pluginBufferA;
        var next = _pluginBufferB;
        CopyMonoToPluginBuffer(_monoBuffer, current, frameCount, pluginChannelCount);

        foreach (var plugin in pluginChain)
        {
            ProcessPluginInBlocks(plugin, current, next, frameCount, pluginChannelCount);
            (current, next) = (next, current);
        }

        DownmixPluginBufferToMonoInto(current, frameCount, pluginChannelCount, _monoBuffer);
    }

    private void ProcessPluginInBlocks(
        VstPluginChainItem plugin,
        float[] input,
        float[] output,
        int frameCount,
        int channelCount)
    {
        var maxBlockSize = Math.Max(1, _maxPluginBlockSize);
        var frameOffset = 0;

        while (frameOffset < frameCount)
        {
            var blockFrameCount = Math.Min(maxBlockSize, frameCount - frameOffset);
            var sampleOffset = frameOffset * channelCount;
            var blockSampleCount = blockFrameCount * channelCount;

            plugin.ProcessFloat32(
                input.AsSpan(sampleOffset, blockSampleCount),
                output.AsSpan(sampleOffset, blockSampleCount),
                blockFrameCount);

            frameOffset += blockFrameCount;
        }
    }

    private VstPluginChainItem[] GetActivePluginChain()
    {
        lock (_pluginChainLock)
        {
            return _activePluginChain;
        }
    }

    private void ClearPluginProcessing(string status)
    {
        lock (_pluginChainLock)
        {
            _activePluginChain = Array.Empty<VstPluginChainItem>();
            _maxPluginBlockSize = 0;
            _pluginProcessingChannels = 0;
            _processingStatus = status;
        }
    }

    private static int GetPluginProcessingChannelCount()
    {
        return 2;
    }

    private static float[] EnsureBuffer(float[] buffer, int requiredLength)
    {
        return buffer.Length >= requiredLength ? buffer : new float[requiredLength];
    }

    private static void DownmixToMonoInto(
        float[] input,
        int frameCount,
        int channelCount,
        float[] destination)
    {
        if (channelCount == 1)
        {
            Array.Copy(input, destination, frameCount);
            return;
        }

        for (var frame = 0; frame < frameCount; frame++)
        {
            var frameOffset = frame * channelCount;
            var sum = 0f;
            for (var channel = 0; channel < channelCount; channel++)
            {
                sum += input[frameOffset + channel];
            }

            destination[frame] = sum / channelCount;
        }
    }

    private static void CopyMonoToPluginBuffer(
        float[] monoSamples,
        float[] pluginInput,
        int frameCount,
        int pluginChannelCount)
    {
        if (pluginChannelCount == 1)
        {
            Array.Copy(monoSamples, pluginInput, frameCount);
            return;
        }

        for (var frame = 0; frame < frameCount; frame++)
        {
            var sample = monoSamples[frame];
            var pluginOffset = frame * pluginChannelCount;
            for (var channel = 0; channel < pluginChannelCount; channel++)
            {
                pluginInput[pluginOffset + channel] = sample;
            }
        }
    }

    private static void DownmixPluginBufferToMonoInto(
        float[] pluginOutput,
        int frameCount,
        int pluginChannelCount,
        float[] destination)
    {
        if (pluginChannelCount == 1)
        {
            Array.Copy(pluginOutput, destination, frameCount);
            return;
        }

        for (var frame = 0; frame < frameCount; frame++)
        {
            var pluginOffset = frame * pluginChannelCount;
            var sum = 0f;
            for (var channel = 0; channel < pluginChannelCount; channel++)
            {
                sum += pluginOutput[pluginOffset + channel];
            }

            destination[frame] = sum / pluginChannelCount;
        }
    }

    private int ConvertMonoToRouteSamples(
        int frameCount,
        int sourceSampleRate,
        WaveFormat routeFormat)
    {
        var monoSource = _monoBuffer;
        var monoCount = frameCount;

        if (sourceSampleRate != routeFormat.SampleRate)
        {
            monoCount = ResampleMonoInto(_monoBuffer, frameCount, sourceSampleRate, routeFormat.SampleRate);
            monoSource = _resampleBuffer;
        }

        var outputSampleCount = monoCount * routeFormat.Channels;
        _routeOutputBuffer = EnsureBuffer(_routeOutputBuffer, outputSampleCount);

        for (var frame = 0; frame < monoCount; frame++)
        {
            var sample = monoSource[frame];
            var outputOffset = frame * routeFormat.Channels;
            for (var channel = 0; channel < routeFormat.Channels; channel++)
            {
                _routeOutputBuffer[outputOffset + channel] = sample;
            }
        }

        return outputSampleCount;
    }

    private int ResampleMonoInto(
        float[] monoSamples,
        int frameCount,
        int sourceSampleRate,
        int outputSampleRate)
    {
        if (frameCount == 0 || sourceSampleRate <= 0 || outputSampleRate <= 0)
        {
            return 0;
        }

        if (!_resamplerHasLastSample)
        {
            _resampleLastSample = monoSamples[0];
            _resamplePosition = 1.0;
            _resamplerHasLastSample = true;
        }

        var estimatedFrameCount = (int)Math.Ceiling(
            frameCount * outputSampleRate / (double)sourceSampleRate) + 4;
        _resampleBuffer = EnsureBuffer(_resampleBuffer, estimatedFrameCount);

        var step = sourceSampleRate / (double)outputSampleRate;
        var extendedLength = frameCount + 1;
        var count = 0;

        while (_resamplePosition <= frameCount && count < _resampleBuffer.Length)
        {
            var leftIndex = (int)Math.Floor(_resamplePosition);
            var rightIndex = Math.Min(leftIndex + 1, extendedLength - 1);
            var fraction = _resamplePosition - leftIndex;
            var leftSample = GetExtendedMonoSample(monoSamples, leftIndex);
            var rightSample = GetExtendedMonoSample(monoSamples, rightIndex);

            _resampleBuffer[count++] = leftSample + (rightSample - leftSample) * (float)fraction;
            _resamplePosition += step;
        }

        _resamplePosition -= frameCount;
        _resampleLastSample = monoSamples[frameCount - 1];
        return count;
    }

    private float GetExtendedMonoSample(float[] monoSamples, int index)
    {
        return index <= 0 ? _resampleLastSample : monoSamples[index - 1];
    }

    private void ResetResamplerState()
    {
        _resamplePosition = 1.0;
        _resampleLastSample = 0f;
        _resamplerHasLastSample = false;
    }

    private static float CalculatePeakLevel(ReadOnlySpan<float> samples)
    {
        var peak = 0f;
        foreach (var sample in samples)
        {
            if (!float.IsFinite(sample))
            {
                continue;
            }

            peak = Math.Max(peak, Math.Abs(sample));
        }

        return Math.Clamp(peak, 0f, 1f);
    }

    private static WaveFormat CreateRouteWaveFormat(WaveFormat outputMixFormat)
    {
        return outputMixFormat.BitsPerSample == 32 && outputMixFormat.Encoding != WaveFormatEncoding.Pcm
            ? outputMixFormat
            : WaveFormat.CreateIeeeFloatWaveFormat(
                outputMixFormat.SampleRate,
                Math.Max(1, outputMixFormat.Channels));
    }

    private static bool HasEnoughSamplesToStart(AudioRouteBuffer routeBuffer)
    {
        var initialBufferedSamples = (int)Math.Ceiling(
            routeBuffer.WaveFormat.SampleRate *
            routeBuffer.WaveFormat.Channels *
            InitialPreloadSeconds);
        return routeBuffer.BufferedSamples >= Math.Max(routeBuffer.WaveFormat.Channels, initialBufferedSamples);
    }

    private static double CalculateCaptureBlockMilliseconds(int byteCount, WaveFormat waveFormat)
    {
        if (waveFormat.SampleRate <= 0)
        {
            return 0;
        }

        return AudioSampleConverter.GetFrameCount(byteCount, waveFormat) * 1000.0 / waveFormat.SampleRate;
    }

    private static int NormalizePluginBlockSize(int pluginBlockSize)
    {
        return pluginBlockSize switch
        {
            64 or 128 or 256 or 512 or 1024 or 2048 or 4096 => pluginBlockSize,
            _ => 512,
        };
    }

    private static (TimeSpan BufferDuration, int OutputLatencyMs) CalculateRouteLatency(
        WaveFormat captureFormat,
        int pluginBlockSize)
    {
        pluginBlockSize = NormalizePluginBlockSize(pluginBlockSize);
        var blockDurationMs = captureFormat.SampleRate <= 0
            ? 10
            : (int)Math.Ceiling(pluginBlockSize * 1000.0 / captureFormat.SampleRate);

        var outputLatencyMs = Math.Clamp(blockDurationMs * 3, 20, 200);
        var bufferDurationMs = Math.Clamp(blockDurationMs * 8, 80, 500);
        return (TimeSpan.FromMilliseconds(bufferDurationMs), outputLatencyMs);
    }

    private void SetOutputPeakLevel(float level)
    {
        lock (_levelLock)
        {
            _outputPeakLevel = Math.Clamp(level, 0, 1);
        }
    }

    private void SetInputPeakLevel(float level)
    {
        lock (_levelLock)
        {
            _inputPeakLevel = Math.Clamp(level, 0, 1);
        }
    }

    public void Dispose()
    {
        Stop();
    }

    [DllImport("avrt.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr AvSetMmThreadCharacteristicsW(string taskName, ref uint taskIndex);

    [DllImport("avrt.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool AvSetMmThreadPriority(IntPtr avrtHandle, int priority);
}
