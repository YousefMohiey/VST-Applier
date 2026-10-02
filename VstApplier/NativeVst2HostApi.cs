using System.Runtime.InteropServices;

namespace VstApplier;

internal static class NativeVst2HostApi
{
    private const string LibraryName = "Vst2HostNative";

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern int Vst2Host_GetApiVersion();

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern IntPtr Vst2Host_Create();

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern void Vst2Host_Destroy(IntPtr host);

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern int Vst2Host_LoadPlugin(IntPtr host, string pluginPath);

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern int Vst2Host_SetupProcessing(
        IntPtr host,
        double sampleRate,
        int maxBlockSize,
        int inputChannels,
        int outputChannels);

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern int Vst2Host_ProcessFloat32(
        IntPtr host,
        [In] float[] inputInterleaved,
        [Out] float[] outputInterleaved,
        int frameCount);

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern int Vst2Host_SaveState(
        IntPtr host,
        IntPtr buffer,
        int bufferSize);

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern int Vst2Host_LoadState(
        IntPtr host,
        IntPtr buffer,
        int bufferSize);

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern int Vst2Host_OpenEditor(IntPtr host, IntPtr parentHwnd);

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern int Vst2Host_GetEditorSize(IntPtr host, out int width, out int height);

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern void Vst2Host_EditorIdle(IntPtr host);

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern void Vst2Host_CloseEditor(IntPtr host);

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern IntPtr Vst2Host_GetLastError(IntPtr host);
}
