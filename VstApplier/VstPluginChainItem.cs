using System.Drawing;

namespace VstApplier;

public sealed class VstPluginChainItem : IDisposable
{
    private readonly object _hostLock = new();
    private bool _disposed;

    public VstPluginChainItem(string name, string path, VstPluginFormat format, IAudioPluginHost host)
    {
        Name = name;
        Path = path;
        Format = format;
        Host = host;
    }

    public string Name { get; }

    public string Path { get; }

    public VstPluginFormat Format { get; }

    public IAudioPluginHost Host { get; }

    public bool IsEnabled { get; set; } = true;

    public void SetupProcessing(double sampleRate, int maxBlockSize, int inputChannels, int outputChannels)
    {
        lock (_hostLock)
        {
            EnsureNotDisposed();
            Host.SetupProcessing(sampleRate, maxBlockSize, inputChannels, outputChannels);
        }
    }

    public void ProcessFloat32(ReadOnlySpan<float> inputInterleaved, Span<float> outputInterleaved, int frameCount)
    {
        lock (_hostLock)
        {
            EnsureNotDisposed();
            Host.ProcessFloat32(inputInterleaved, outputInterleaved, frameCount);
        }
    }

    public byte[]? SaveState()
    {
        lock (_hostLock)
        {
            EnsureNotDisposed();
            return Host.SaveState();
        }
    }

    public void LoadState(byte[] state)
    {
        lock (_hostLock)
        {
            EnsureNotDisposed();
            Host.LoadState(state);
        }
    }

    public void OpenEditor(IntPtr parentHwnd)
    {
        lock (_hostLock)
        {
            EnsureNotDisposed();
            Host.OpenEditor(parentHwnd);
        }
    }

    public Size? GetEditorSize()
    {
        lock (_hostLock)
        {
            EnsureNotDisposed();
            return Host.GetEditorSize();
        }
    }

    public void EditorIdle()
    {
        lock (_hostLock)
        {
            if (_disposed)
            {
                return;
            }

            Host.EditorIdle();
        }
    }

    public void CloseEditor()
    {
        lock (_hostLock)
        {
            if (_disposed)
            {
                return;
            }

            Host.CloseEditor();
        }
    }

    public void Dispose()
    {
        lock (_hostLock)
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;
            Host.Dispose();
        }
    }

    public override string ToString() => Format == VstPluginFormat.Vst2
        ? $"{Name} (VST2 x64)"
        : Name;

    private void EnsureNotDisposed()
    {
        if (_disposed)
        {
            throw new ObjectDisposedException(nameof(VstPluginChainItem));
        }
    }
}
