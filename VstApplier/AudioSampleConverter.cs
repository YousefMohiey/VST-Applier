namespace VstApplier;

using System.Buffers.Binary;
using NAudio.Wave;

public static class AudioSampleConverter
{
    public static bool IsSupported(WaveFormat waveFormat)
    {
        return waveFormat.BitsPerSample switch
        {
            16 or 24 => true,
            32 => true,
            _ => false,
        };
    }

    public static int GetFrameCount(int byteCount, WaveFormat waveFormat)
    {
        if (waveFormat.BlockAlign <= 0 || byteCount % waveFormat.BlockAlign != 0)
        {
            throw new InvalidOperationException("Captured audio block is not frame-aligned.");
        }

        return byteCount / waveFormat.BlockAlign;
    }

    public static void ConvertToFloat32(
        ReadOnlySpan<byte> source,
        WaveFormat waveFormat,
        Span<float> destination)
    {
        var sampleCount = GetExpectedSampleCount(source.Length, waveFormat);

        if (destination.Length < sampleCount)
        {
            throw new ArgumentException("Destination float buffer is too small.", nameof(destination));
        }

        switch (waveFormat.BitsPerSample)
        {
            case 16:
                ConvertPcm16ToFloat32(source, destination[..sampleCount]);
                break;
            case 24:
                ConvertPcm24ToFloat32(source, destination[..sampleCount]);
                break;
            case 32 when waveFormat.Encoding == WaveFormatEncoding.Pcm:
                ConvertPcm32ToFloat32(source, destination[..sampleCount]);
                break;
            case 32:
                ConvertIeeeFloat32ToFloat32(source, destination[..sampleCount]);
                break;
            default:
                throw new NotSupportedException($"Unsupported capture format: {waveFormat.BitsPerSample}-bit {waveFormat.Encoding}.");
        }
    }

    public static void ConvertFromFloat32(
        ReadOnlySpan<float> source,
        WaveFormat waveFormat,
        Span<byte> destination)
    {
        var sampleCount = GetExpectedSampleCount(destination.Length, waveFormat);

        if (source.Length < sampleCount)
        {
            throw new ArgumentException("Source float buffer is too small.", nameof(source));
        }

        switch (waveFormat.BitsPerSample)
        {
            case 16:
                ConvertFloat32ToPcm16(source[..sampleCount], destination);
                break;
            case 24:
                ConvertFloat32ToPcm24(source[..sampleCount], destination);
                break;
            case 32 when waveFormat.Encoding == WaveFormatEncoding.Pcm:
                ConvertFloat32ToPcm32(source[..sampleCount], destination);
                break;
            case 32:
                ConvertFloat32ToIeeeFloat32(source[..sampleCount], destination);
                break;
            default:
                throw new NotSupportedException($"Unsupported capture format: {waveFormat.BitsPerSample}-bit {waveFormat.Encoding}.");
        }
    }

    private static int GetExpectedSampleCount(int byteCount, WaveFormat waveFormat)
    {
        var frameCount = GetFrameCount(byteCount, waveFormat);
        return frameCount * waveFormat.Channels;
    }

    private static void ConvertIeeeFloat32ToFloat32(ReadOnlySpan<byte> source, Span<float> destination)
    {
        for (var sampleIndex = 0; sampleIndex < destination.Length; sampleIndex++)
        {
            destination[sampleIndex] = BinaryPrimitives.ReadSingleLittleEndian(source.Slice(sampleIndex * 4, 4));
        }
    }

    private static void ConvertPcm16ToFloat32(ReadOnlySpan<byte> source, Span<float> destination)
    {
        for (var sampleIndex = 0; sampleIndex < destination.Length; sampleIndex++)
        {
            var sample = BinaryPrimitives.ReadInt16LittleEndian(source.Slice(sampleIndex * 2, 2));
            destination[sampleIndex] = sample / 32768f;
        }
    }

    private static void ConvertPcm24ToFloat32(ReadOnlySpan<byte> source, Span<float> destination)
    {
        for (var sampleIndex = 0; sampleIndex < destination.Length; sampleIndex++)
        {
            var offset = sampleIndex * 3;
            var sample = source[offset] | source[offset + 1] << 8 | source[offset + 2] << 16;
            if ((sample & 0x800000) != 0)
            {
                sample |= unchecked((int)0xFF000000);
            }

            destination[sampleIndex] = sample / 8388608f;
        }
    }

    private static void ConvertPcm32ToFloat32(ReadOnlySpan<byte> source, Span<float> destination)
    {
        for (var sampleIndex = 0; sampleIndex < destination.Length; sampleIndex++)
        {
            var sample = BinaryPrimitives.ReadInt32LittleEndian(source.Slice(sampleIndex * 4, 4));
            destination[sampleIndex] = sample / 2147483648f;
        }
    }

    private static void ConvertFloat32ToIeeeFloat32(ReadOnlySpan<float> source, Span<byte> destination)
    {
        for (var sampleIndex = 0; sampleIndex < source.Length; sampleIndex++)
        {
            BinaryPrimitives.WriteSingleLittleEndian(
                destination.Slice(sampleIndex * 4, 4),
                SanitizeFloat(source[sampleIndex]));
        }
    }

    private static void ConvertFloat32ToPcm16(ReadOnlySpan<float> source, Span<byte> destination)
    {
        for (var sampleIndex = 0; sampleIndex < source.Length; sampleIndex++)
        {
            BinaryPrimitives.WriteInt16LittleEndian(
                destination.Slice(sampleIndex * 2, 2),
                (short)FloatToSignedPcm(source[sampleIndex], 32768, 32767));
        }
    }

    private static void ConvertFloat32ToPcm24(ReadOnlySpan<float> source, Span<byte> destination)
    {
        for (var sampleIndex = 0; sampleIndex < source.Length; sampleIndex++)
        {
            var sample = FloatToSignedPcm(source[sampleIndex], 8388608, 8388607);
            var offset = sampleIndex * 3;
            destination[offset] = (byte)(sample & 0xFF);
            destination[offset + 1] = (byte)((sample >> 8) & 0xFF);
            destination[offset + 2] = (byte)((sample >> 16) & 0xFF);
        }
    }

    private static void ConvertFloat32ToPcm32(ReadOnlySpan<float> source, Span<byte> destination)
    {
        for (var sampleIndex = 0; sampleIndex < source.Length; sampleIndex++)
        {
            BinaryPrimitives.WriteInt32LittleEndian(
                destination.Slice(sampleIndex * 4, 4),
                FloatToSignedPcm(source[sampleIndex], 2147483648L, 2147483647));
        }
    }

    private static int FloatToSignedPcm(float sample, long negativeScale, int positiveMax)
    {
        sample = SanitizeFloat(sample);

        if (sample <= -1f)
        {
            return (int)-negativeScale;
        }

        if (sample >= 1f)
        {
            return positiveMax;
        }

        return (int)MathF.Round(sample * positiveMax);
    }

    private static float SanitizeFloat(float sample)
    {
        return float.IsFinite(sample) ? sample : 0f;
    }
}
