using System.Drawing;

namespace VstApplier;

public interface IAudioPluginHost : IDisposable
{
    void LoadPlugin(string pluginPath);

    void SetupProcessing(double sampleRate, int maxBlockSize, int inputChannels, int outputChannels);

    void ProcessFloat32(ReadOnlySpan<float> inputInterleaved, Span<float> outputInterleaved, int frameCount);

    byte[]? SaveState();

    void LoadState(byte[] state);

    void OpenEditor(IntPtr parentHwnd);

    Size? GetEditorSize();

    void EditorIdle();

    void CloseEditor();
}
