#pragma once

#ifdef VST3HOSTNATIVE_EXPORTS
#define VST3HOST_API __declspec(dllexport)
#else
#define VST3HOST_API __declspec(dllimport)
#endif

typedef void* Vst3HostInstanceHandle;

extern "C"
{
VST3HOST_API int Vst3Host_GetApiVersion();

VST3HOST_API Vst3HostInstanceHandle Vst3Host_Create();

VST3HOST_API void Vst3Host_Destroy(Vst3HostInstanceHandle host);

VST3HOST_API int Vst3Host_LoadPlugin(
    Vst3HostInstanceHandle host,
    const wchar_t* pluginPath);

VST3HOST_API int Vst3Host_SetupProcessing(
    Vst3HostInstanceHandle host,
    double sampleRate,
    int maxBlockSize,
    int inputChannels,
    int outputChannels);

VST3HOST_API int Vst3Host_ProcessFloat32(
    Vst3HostInstanceHandle host,
    const float* inputInterleaved,
    float* outputInterleaved,
    int frameCount);

VST3HOST_API int Vst3Host_SaveState(
    Vst3HostInstanceHandle host,
    void* buffer,
    int bufferSize);

VST3HOST_API int Vst3Host_LoadState(
    Vst3HostInstanceHandle host,
    const void* buffer,
    int bufferSize);

VST3HOST_API int Vst3Host_OpenEditor(
    Vst3HostInstanceHandle host,
    void* parentHwnd);

VST3HOST_API void Vst3Host_CloseEditor(Vst3HostInstanceHandle host);

VST3HOST_API const wchar_t* Vst3Host_GetLastError(Vst3HostInstanceHandle host);
}
