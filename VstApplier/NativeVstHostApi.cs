using System.Runtime.InteropServices;

namespace VstApplier;

internal static class NativeVstHostApi
{
    private const string LibraryName = "Vst3HostNative";

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern int Vst3Host_GetApiVersion();

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern IntPtr Vst3Host_Create();

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern void Vst3Host_Destroy(IntPtr host);

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern int Vst3Host_LoadPlugin(IntPtr host, string pluginPath);

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern int Vst3Host_SetupProcessing(
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
    internal static extern int Vst3Host_ProcessFloat32(
        IntPtr host,
        [In] float[] inputInterleaved,
        [Out] float[] outputInterleaved,
        int frameCount);

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern int Vst3Host_SaveState(
        IntPtr host,
        IntPtr buffer,
        int bufferSize);

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern int Vst3Host_LoadState(
        IntPtr host,
        IntPtr buffer,
        int bufferSize);

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern int Vst3Host_OpenEditor(IntPtr host, IntPtr parentHwnd);

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern void Vst3Host_CloseEditor(IntPtr host);

    [DllImport(
        LibraryName,
        CallingConvention = CallingConvention.Cdecl,
        CharSet = CharSet.Unicode,
        ExactSpelling = true)]
    internal static extern IntPtr Vst3Host_GetLastError(IntPtr host);
}
