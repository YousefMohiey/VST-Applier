using System.Drawing;
using System.Runtime.InteropServices;

namespace VstApplier;

public sealed class NativeVst2Host : IAudioPluginHost
{
    private IntPtr _handle;
    private int _inputChannels;
    private int _outputChannels;
    private bool _disposed;

    public NativeVst2Host()
    {
        try
        {
            _handle = NativeVst2HostApi.Vst2Host_Create();
        }
        catch (Exception ex) when (IsNativeLoadException(ex))
        {
            throw CreateUnavailableException(ex);
        }

        if (_handle == IntPtr.Zero)
        {
            throw new NativeVstHostException("Native VST2 host could not be created.");
        }
    }

    ~NativeVst2Host()
    {
        Dispose(false);
    }

    public static int ApiVersion
    {
        get
        {
            try
            {
                return NativeVst2HostApi.Vst2Host_GetApiVersion();
            }
            catch (Exception ex) when (IsNativeLoadException(ex))
            {
                throw CreateUnavailableException(ex);
            }
        }
    }

    public void LoadPlugin(string pluginPath)
    {
        EnsureNotDisposed();

        try
        {
            ThrowIfFailed(NativeVst2HostApi.Vst2Host_LoadPlugin(_handle, pluginPath));
        }
        catch (Exception ex) when (IsNativeLoadException(ex))
        {
            throw CreateUnavailableException(ex);
        }
    }

    public void SetupProcessing(double sampleRate, int maxBlockSize, int inputChannels, int outputChannels)
    {
        EnsureNotDisposed();

        try
        {
            ThrowIfFailed(NativeVst2HostApi.Vst2Host_SetupProcessing(
                _handle,
                sampleRate,
                maxBlockSize,
                inputChannels,
                outputChannels));
        }
        catch (Exception ex) when (IsNativeLoadException(ex))
        {
            throw CreateUnavailableException(ex);
        }

        _inputChannels = inputChannels;
        _outputChannels = outputChannels;
    }

    public void ProcessFloat32(
        ReadOnlySpan<float> inputInterleaved,
        Span<float> outputInterleaved,
        int frameCount)
    {
        EnsureNotDisposed();

        if (frameCount < 0)
        {
            throw new NativeVstHostException("Frame count must not be negative.");
        }

        var inputSampleCount = frameCount * _inputChannels;
        var outputSampleCount = frameCount * _outputChannels;

        if (inputInterleaved.Length < inputSampleCount || outputInterleaved.Length < outputSampleCount)
        {
            throw new NativeVstHostException("Input or output buffer is too small for the configured VST2 channel count.");
        }

        var input = inputInterleaved[..inputSampleCount].ToArray();
        var output = new float[outputSampleCount];

        try
        {
            ThrowIfFailed(NativeVst2HostApi.Vst2Host_ProcessFloat32(_handle, input, output, frameCount));
        }
        catch (Exception ex) when (IsNativeLoadException(ex))
        {
            throw CreateUnavailableException(ex);
        }

        output.CopyTo(outputInterleaved);
    }

    public byte[]? SaveState()
    {
        EnsureNotDisposed();

        int requiredSize;
        try
        {
            requiredSize = NativeVst2HostApi.Vst2Host_SaveState(_handle, IntPtr.Zero, 0);
        }
        catch (Exception ex) when (IsNativeLoadException(ex))
        {
            throw CreateUnavailableException(ex);
        }

        if (requiredSize < 0)
        {
            var message = GetLastError();
            throw new NativeVstHostException(string.IsNullOrWhiteSpace(message)
                ? $"Native VST2 host state save failed with code {requiredSize}."
                : message);
        }

        if (requiredSize == 0)
        {
            return null;
        }

        var stateBuffer = new byte[requiredSize];
        var nativeBuffer = Marshal.AllocHGlobal(requiredSize);
        try
        {
            int writtenSize;
            try
            {
                writtenSize = NativeVst2HostApi.Vst2Host_SaveState(_handle, nativeBuffer, requiredSize);
            }
            catch (Exception ex) when (IsNativeLoadException(ex))
            {
                throw CreateUnavailableException(ex);
            }

            if (writtenSize < 0)
            {
                var message = GetLastError();
                throw new NativeVstHostException(string.IsNullOrWhiteSpace(message)
                    ? $"Native VST2 host state save failed with code {writtenSize}."
                    : message);
            }

            if (writtenSize != requiredSize)
            {
                throw new NativeVstHostException("Native VST2 host state size changed between save calls.");
            }

            Marshal.Copy(nativeBuffer, stateBuffer, 0, requiredSize);
        }
        finally
        {
            Marshal.FreeHGlobal(nativeBuffer);
        }

        return stateBuffer;
    }

    public void LoadState(byte[] state)
    {
        EnsureNotDisposed();
        ArgumentNullException.ThrowIfNull(state);

        if (state.Length == 0)
        {
            return;
        }

        var nativeBuffer = Marshal.AllocHGlobal(state.Length);
        try
        {
            Marshal.Copy(state, 0, nativeBuffer, state.Length);

            try
            {
                ThrowIfFailed(NativeVst2HostApi.Vst2Host_LoadState(_handle, nativeBuffer, state.Length));
            }
            catch (Exception ex) when (IsNativeLoadException(ex))
            {
                throw CreateUnavailableException(ex);
            }
        }
        finally
        {
            Marshal.FreeHGlobal(nativeBuffer);
        }
    }

    public void OpenEditor(IntPtr parentHwnd)
    {
        EnsureNotDisposed();

        try
        {
            ThrowIfFailed(NativeVst2HostApi.Vst2Host_OpenEditor(_handle, parentHwnd));
        }
        catch (Exception ex) when (IsNativeLoadException(ex))
        {
            throw CreateUnavailableException(ex);
        }
    }

    public Size? GetEditorSize()
    {
        EnsureNotDisposed();

        try
        {
            ThrowIfFailed(NativeVst2HostApi.Vst2Host_GetEditorSize(_handle, out var width, out var height));
            return width > 0 && height > 0
                ? new Size(width, height)
                : null;
        }
        catch (NativeVstHostException)
        {
            return null;
        }
        catch (Exception ex) when (IsNativeLoadException(ex))
        {
            throw CreateUnavailableException(ex);
        }
    }

    public void EditorIdle()
    {
        if (_disposed || _handle == IntPtr.Zero)
        {
            return;
        }

        try
        {
            NativeVst2HostApi.Vst2Host_EditorIdle(_handle);
        }
        catch (Exception ex) when (IsNativeLoadException(ex))
        {
            throw CreateUnavailableException(ex);
        }
    }

    public void CloseEditor()
    {
        if (_disposed || _handle == IntPtr.Zero)
        {
            return;
        }

        try
        {
            NativeVst2HostApi.Vst2Host_CloseEditor(_handle);
        }
        catch (Exception ex) when (IsNativeLoadException(ex))
        {
            throw CreateUnavailableException(ex);
        }
    }

    public void Dispose()
    {
        Dispose(true);
        GC.SuppressFinalize(this);
    }

    private void Dispose(bool disposing)
    {
        if (_disposed)
        {
            return;
        }

        var handle = _handle;
        _handle = IntPtr.Zero;
        _disposed = true;

        if (handle == IntPtr.Zero)
        {
            return;
        }

        try
        {
            NativeVst2HostApi.Vst2Host_Destroy(handle);
        }
        catch
        {
        }
    }

    private void ThrowIfFailed(int result)
    {
        if (result == 0)
        {
            return;
        }

        var message = GetLastError();
        throw new NativeVstHostException(string.IsNullOrWhiteSpace(message)
            ? $"Native VST2 host call failed with code {result}."
            : message);
    }

    private string GetLastError()
    {
        try
        {
            var errorPointer = NativeVst2HostApi.Vst2Host_GetLastError(_handle);
            return Marshal.PtrToStringUni(errorPointer) ?? string.Empty;
        }
        catch (Exception ex) when (IsNativeLoadException(ex))
        {
            return CreateUnavailableException(ex).Message;
        }
    }

    private void EnsureNotDisposed()
    {
        if (_disposed)
        {
            throw new ObjectDisposedException(nameof(NativeVst2Host));
        }
    }

    private static bool IsNativeLoadException(Exception ex)
    {
        return ex is DllNotFoundException ||
            ex is EntryPointNotFoundException ||
            ex is BadImageFormatException;
    }

    private static NativeVstHostException CreateUnavailableException(Exception ex)
    {
        return new NativeVstHostException($"Native VST2 host unavailable: {ex.Message}", ex);
    }
}
