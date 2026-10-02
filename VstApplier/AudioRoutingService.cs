namespace VstApplier;

using NAudio.CoreAudioApi;
using NAudio.Wave;

public sealed class AudioRoutingService : IDisposable
{
    private const double InitialPreloadSeconds = 0.10;

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
            ResetResamplerState();
            ClearPluginProcessing("VST bypassed");
            SetInputPeakLevel(0);
            SetOutputPeakLevel(0);
        }
    }

    private void Capture_DataAvailable(object? sender, WaveInEventArgs e)
    {
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

        var outputSamples = ProcessCaptureBlockToRouteSamples(
            e.Buffer,
            e.BytesRecorded,
            _capture.WaveFormat,
            _routeBuffer.WaveFormat);

        SetOutputPeakLevel(CalculatePeakLevel(outputSamples));
        _routeBuffer.AddSamples(outputSamples);

        if (!_outputStarted && HasEnoughSamplesToStart(_routeBuffer))
        {
            _output.Play();
            _outputStarted = true;
        }
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

    private float[] ProcessCaptureBlockToRouteSamples(
        byte[] sourceBuffer,
        int byteCount,
        WaveFormat captureFormat,
        WaveFormat routeFormat)
    {
        var frameCount = AudioSampleConverter.GetFrameCount(byteCount, captureFormat);
        var channelCount = captureFormat.Channels;
        var captureSampleCount = frameCount * channelCount;
        var captureInput = new float[captureSampleCount];

        AudioSampleConverter.ConvertToFloat32(
            sourceBuffer.AsSpan(0, byteCount),
            captureFormat,
            captureInput);

        var monoSamples = DownmixToMono(captureInput, frameCount, channelCount);
        SetInputPeakLevel(CalculatePeakLevel(monoSamples));
        var pluginChain = GetActivePluginChain();

        if (pluginChain.Length > 0)
        {
            try
            {
                monoSamples = ProcessMonoWithPluginChain(monoSamples, pluginChain);
            }
            catch (Exception ex)
            {
                ClearPluginProcessing($"VST bypassed: {ex.Message}");
            }
        }

        return ConvertMonoToRouteSamples(monoSamples, captureFormat.SampleRate, routeFormat);
    }

    private float[] ProcessMonoWithPluginChain(
        float[] monoSamples,
        IReadOnlyList<VstPluginChainItem> pluginChain)
    {
        var frameCount = monoSamples.Length;
        var pluginChannelCount = _pluginProcessingChannels <= 0
            ? GetPluginProcessingChannelCount()
            : _pluginProcessingChannels;
        var pluginSampleCount = frameCount * pluginChannelCount;
        var current = new float[pluginSampleCount];
        var next = new float[pluginSampleCount];

        CopyMonoToPluginBuffer(monoSamples, current, frameCount, pluginChannelCount);

        foreach (var plugin in pluginChain)
        {
            ProcessPluginInBlocks(plugin, current, next, frameCount, pluginChannelCount);
            (current, next) = (next, current);
        }

        return DownmixPluginBufferToMono(current, frameCount, pluginChannelCount);
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

    private static float[] DownmixToMono(
        float[] input,
        int frameCount,
        int channelCount)
    {
        var monoSamples = new float[frameCount];

        if (channelCount == 1)
        {
            Array.Copy(input, monoSamples, frameCount);
            return monoSamples;
        }

        for (var frame = 0; frame < frameCount; frame++)
        {
            var frameOffset = frame * channelCount;
            var sum = 0f;
            for (var channel = 0; channel < channelCount; channel++)
            {
                sum += input[frameOffset + channel];
            }

            monoSamples[frame] = sum / channelCount;
        }

        return monoSamples;
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

    private static float[] DownmixPluginBufferToMono(
        float[] pluginOutput,
        int frameCount,
        int pluginChannelCount)
    {
        var monoSamples = new float[frameCount];

        if (pluginChannelCount == 1)
        {
            Array.Copy(pluginOutput, monoSamples, frameCount);
            return monoSamples;
        }

        for (var frame = 0; frame < frameCount; frame++)
        {
            var pluginOffset = frame * pluginChannelCount;
            var sum = 0f;
            for (var channel = 0; channel < pluginChannelCount; channel++)
            {
                sum += pluginOutput[pluginOffset + channel];
            }

            monoSamples[frame] = sum / pluginChannelCount;
        }

        return monoSamples;
    }

    private float[] ConvertMonoToRouteSamples(
        float[] monoSamples,
        int sourceSampleRate,
        WaveFormat routeFormat)
    {
        var routeMonoSamples = sourceSampleRate == routeFormat.SampleRate
            ? monoSamples
            : ResampleMono(monoSamples, sourceSampleRate, routeFormat.SampleRate);

        var outputSamples = new float[routeMonoSamples.Length * routeFormat.Channels];
        for (var frame = 0; frame < routeMonoSamples.Length; frame++)
        {
            var sample = routeMonoSamples[frame];
            var outputOffset = frame * routeFormat.Channels;
            for (var channel = 0; channel < routeFormat.Channels; channel++)
            {
                outputSamples[outputOffset + channel] = sample;
            }
        }

        return outputSamples;
    }

    private float[] ResampleMono(
        float[] monoSamples,
        int sourceSampleRate,
        int outputSampleRate)
    {
        if (monoSamples.Length == 0 || sourceSampleRate <= 0 || outputSampleRate <= 0)
        {
            return Array.Empty<float>();
        }

        if (!_resamplerHasLastSample)
        {
            _resampleLastSample = monoSamples[0];
            _resamplePosition = 1.0;
            _resamplerHasLastSample = true;
        }

        var estimatedFrameCount = (int)Math.Ceiling(
            monoSamples.Length * outputSampleRate / (double)sourceSampleRate) + 2;
        var output = new List<float>(estimatedFrameCount);
        var step = sourceSampleRate / (double)outputSampleRate;
        var extendedLength = monoSamples.Length + 1;

        while (_resamplePosition <= monoSamples.Length)
        {
            var leftIndex = (int)Math.Floor(_resamplePosition);
            var rightIndex = Math.Min(leftIndex + 1, extendedLength - 1);
            var fraction = _resamplePosition - leftIndex;
            var leftSample = GetExtendedMonoSample(monoSamples, leftIndex);
            var rightSample = GetExtendedMonoSample(monoSamples, rightIndex);

            output.Add(leftSample + (rightSample - leftSample) * (float)fraction);
            _resamplePosition += step;
        }

        _resamplePosition -= monoSamples.Length;
        _resampleLastSample = monoSamples[^1];
        return output.ToArray();
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

    private static float CalculatePeakLevel(float[] samples)
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
}
