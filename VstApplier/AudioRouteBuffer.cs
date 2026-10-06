namespace VstApplier;

using System.Buffers.Binary;
using NAudio.Wave;

public sealed class AudioRouteBuffer : IWaveProvider
{
    private readonly object _lock = new();
    private readonly float[] _buffer;
    private int _readPosition;
    private int _writePosition;
    private int _bufferedSamples;

    public AudioRouteBuffer(WaveFormat waveFormat, TimeSpan bufferDuration)
    {
        WaveFormat = waveFormat;
        var capacitySamples = (int)Math.Ceiling(
            waveFormat.SampleRate *
            Math.Max(0.1, bufferDuration.TotalSeconds) *
            waveFormat.Channels);
        _buffer = new float[Math.Max(waveFormat.Channels, capacitySamples)];
    }

    public WaveFormat WaveFormat { get; }

    public int BufferedSamples
    {
        get
        {
            lock (_lock)
            {
                return _bufferedSamples;
            }
        }
    }

    public double BufferedMilliseconds
    {
        get
        {
            lock (_lock)
            {
                return SamplesToMilliseconds(_bufferedSamples);
            }
        }
    }

    public double CapacityMilliseconds => SamplesToMilliseconds(_buffer.Length);

    public void AddSamples(ReadOnlySpan<float> samples)
    {
        lock (_lock)
        {
            if (samples.Length >= _buffer.Length)
            {
                samples = samples[^_buffer.Length..];
                _readPosition = 0;
                _writePosition = 0;
                _bufferedSamples = 0;
            }

            var overflowSamples = Math.Max(0, _bufferedSamples + samples.Length - _buffer.Length);
            if (overflowSamples > 0)
            {
                _readPosition = (_readPosition + overflowSamples) % _buffer.Length;
                _bufferedSamples -= overflowSamples;
            }

            foreach (var sample in samples)
            {
                _buffer[_writePosition] = float.IsFinite(sample) ? Math.Clamp(sample, -1f, 1f) : 0f;
                _writePosition = (_writePosition + 1) % _buffer.Length;
            }

            _bufferedSamples += samples.Length;
        }
    }

    /// <summary>
    /// Drops the oldest buffered audio so the remaining backlog matches the target.
    /// Used by the routing service to keep the delay bounded when the CPU is busy;
    /// without this, a capture burst leaves a permanent delay in the route.
    /// </summary>
    public void TrimOldestMilliseconds(double targetMilliseconds)
    {
        lock (_lock)
        {
            var targetSamples = MillisecondsToSamples(targetMilliseconds);
            var excessSamples = _bufferedSamples - targetSamples;
            if (excessSamples <= 0)
            {
                return;
            }

            _readPosition = (_readPosition + excessSamples) % _buffer.Length;
            _bufferedSamples -= excessSamples;
        }
    }

    public int Read(byte[] buffer, int offset, int count)
    {
        var samplesRequested = count / sizeof(float);
        var bytesWritten = samplesRequested * sizeof(float);

        lock (_lock)
        {
            for (var sampleIndex = 0; sampleIndex < samplesRequested; sampleIndex++)
            {
                var sample = 0f;
                if (_bufferedSamples > 0)
                {
                    sample = _buffer[_readPosition];
                    _readPosition = (_readPosition + 1) % _buffer.Length;
                    _bufferedSamples--;
                }

                BinaryPrimitives.WriteSingleLittleEndian(
                    buffer.AsSpan(offset + sampleIndex * sizeof(float), sizeof(float)),
                    sample);
            }
        }

        if (bytesWritten < count)
        {
            Array.Clear(buffer, offset + bytesWritten, count - bytesWritten);
        }

        return count;
    }

    private double SamplesToMilliseconds(int sampleCount)
    {
        var samplesPerSecond = WaveFormat.SampleRate * WaveFormat.Channels;
        return samplesPerSecond <= 0
            ? 0
            : sampleCount * 1000.0 / samplesPerSecond;
    }

    private int MillisecondsToSamples(double milliseconds)
    {
        return (int)Math.Ceiling(
            WaveFormat.SampleRate * WaveFormat.Channels * milliseconds / 1000.0);
    }
}
