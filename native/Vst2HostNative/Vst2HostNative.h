#pragma once

#ifdef VST2HOSTNATIVE_EXPORTS
#define VST2HOST_API __declspec(dllexport)
#else
#define VST2HOST_API __declspec(dllimport)
#endif

typedef void* Vst2HostInstanceHandle;

extern "C"
{
VST2HOST_API int Vst2Host_GetApiVersion();

VST2HOST_API Vst2HostInstanceHandle Vst2Host_Create();

VST2HOST_API void Vst2Host_Destroy(Vst2HostInstanceHandle host);

VST2HOST_API int Vst2Host_LoadPlugin(
    Vst2HostInstanceHandle host,
    const wchar_t* pluginPath);

VST2HOST_API int Vst2Host_SetupProcessing(
    Vst2HostInstanceHandle host,
    double sampleRate,
    int maxBlockSize,
    int inputChannels,
    int outputChannels);

VST2HOST_API int Vst2Host_ProcessFloat32(
    Vst2HostInstanceHandle host,
    const float* inputInterleaved,
    float* outputInterleaved,
    int frameCount);

VST2HOST_API int Vst2Host_SaveState(
    Vst2HostInstanceHandle host,
    void* buffer,
    int bufferSize);

VST2HOST_API int Vst2Host_LoadState(
    Vst2HostInstanceHandle host,
    const void* buffer,
    int bufferSize);

VST2HOST_API int Vst2Host_OpenEditor(
    Vst2HostInstanceHandle host,
    void* parentHwnd);

VST2HOST_API int Vst2Host_GetEditorSize(
    Vst2HostInstanceHandle host,
    int* width,
    int* height);

VST2HOST_API void Vst2Host_EditorIdle(Vst2HostInstanceHandle host);

VST2HOST_API void Vst2Host_CloseEditor(Vst2HostInstanceHandle host);

VST2HOST_API const wchar_t* Vst2Host_GetLastError(Vst2HostInstanceHandle host);
}
