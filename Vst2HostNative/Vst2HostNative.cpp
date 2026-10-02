#include "Vst2HostNative.h"

#include <algorithm>
#include <cstdint>
#include <cstring>
#include <memory>
#include <string>
#include <string_view>
#include <vector>
#include <windows.h>

namespace
{
constexpr int kOk = 0;
constexpr int kErrorInvalidArgument = -1;
constexpr int kErrorFileNotFound = -2;
constexpr int kErrorLoadLibrary = -3;
constexpr int kErrorEntryPoint = -4;
constexpr int kErrorPluginLoad = -5;
constexpr int kErrorUnsupported = -6;
constexpr int kErrorNotConfigured = -7;

constexpr uint32_t kChunkStateBlobMagic = 0x43545356; // 'VSTC'
constexpr uint32_t kParamStateBlobMagic = 0x44545356; // 'VSTD'
constexpr uint32_t kLegacyChunkStateBlobMagic = 0x434A4E53; // 'SNJC'
constexpr uint32_t kLegacyParamStateBlobMagic = 0x504A4E53; // 'SNJP'
constexpr uint32_t kStateBlobVersion = 1;

constexpr int32_t kEffectMagic = 0x56737450; // 'VstP'
constexpr int32_t effOpen = 0;
constexpr int32_t effClose = 1;
constexpr int32_t effSetSampleRate = 10;
constexpr int32_t effSetBlockSize = 11;
constexpr int32_t effMainsChanged = 12;
constexpr int32_t effEditGetRect = 13;
constexpr int32_t effEditOpen = 14;
constexpr int32_t effEditClose = 15;
constexpr int32_t effEditIdle = 19;
constexpr int32_t effGetEffectName = 45;
constexpr int32_t effGetVendorString = 47;
constexpr int32_t effGetProductString = 48;
constexpr int32_t effGetChunk = 23;
constexpr int32_t effSetChunk = 24;

constexpr int32_t effFlagsHasEditor = 1 << 0;
constexpr int32_t effFlagsCanReplacing = 1 << 4;

constexpr int32_t audioMasterAutomate = 0;
constexpr int32_t audioMasterVersion = 1;
constexpr int32_t audioMasterGetSampleRate = 16;
constexpr int32_t audioMasterGetBlockSize = 17;
constexpr int32_t audioMasterGetInputLatency = 18;
constexpr int32_t audioMasterGetOutputLatency = 19;
constexpr int32_t audioMasterCanDo = 37;
constexpr int32_t audioMasterGetLanguage = 38;
constexpr int32_t audioMasterCockosExtension = static_cast<int32_t>(0xDEADBEEF);
constexpr int32_t cockosGetApiFunction = static_cast<int32_t>(0xDEADF00D);
constexpr int32_t cockosGetHostContext = static_cast<int32_t>(0xDEADF00E);

struct AEffect;

using AudioMasterCallback = intptr_t(__cdecl*)(
    AEffect* effect,
    int32_t opcode,
    int32_t index,
    intptr_t value,
    void* ptr,
    float opt);

using AEffectDispatcherProc = intptr_t(__cdecl*)(
    AEffect* effect,
    int32_t opcode,
    int32_t index,
    intptr_t value,
    void* ptr,
    float opt);

using AEffectProcessProc = void(__cdecl*)(
    AEffect* effect,
    float** inputs,
    float** outputs,
    int32_t sampleFrames);

using AEffectSetParameterProc = void(__cdecl*)(
    AEffect* effect,
    int32_t index,
    float parameter);

using AEffectGetParameterProc = float(__cdecl*)(
    AEffect* effect,
    int32_t index);

struct AEffect
{
    int32_t magic;
    AEffectDispatcherProc dispatcher;
    AEffectProcessProc process;
    AEffectSetParameterProc setParameter;
    AEffectGetParameterProc getParameter;
    int32_t numPrograms;
    int32_t numParams;
    int32_t numInputs;
    int32_t numOutputs;
    int32_t flags;
    intptr_t reserved1;
    intptr_t reserved2;
    int32_t initialDelay;
    int32_t realQualities;
    int32_t offQualities;
    float ioRatio;
    void* object;
    void* user;
    int32_t uniqueID;
    int32_t version;
    AEffectProcessProc processReplacing;
    AEffectProcessProc processDoubleReplacing;
    char future[56];
};

struct ERect
{
    int16_t top;
    int16_t left;
    int16_t bottom;
    int16_t right;
};

using VstPluginMainProc = AEffect*(__cdecl*)(AudioMasterCallback audioMaster);

thread_local std::vector<int32_t> g_audioMasterOpcodes;
thread_local std::vector<std::wstring> g_cockosFunctionRequests;

std::wstring ToWide(const char* value);

int g_reaperConfigNumCpu = 1;
char g_reaperFxLoadStateContext = 0;
char g_emptyReaperIniPath[] = "";

double __cdecl CockosGetZeroTime()
{
    return 0.0;
}

int __cdecl CockosGetStoppedPlayState()
{
    return 0;
}

int __cdecl CockosGetStoppedPlayStateEx(void*)
{
    return 0;
}

void __cdecl CockosSetEditCursorPosition(double, bool, bool)
{
}

int __cdecl CockosGetSetRepeat(int)
{
    return 0;
}

void __cdecl CockosGetProjectPath(char* buffer, int bufferSize)
{
    if (buffer != nullptr && bufferSize > 0)
    {
        buffer[0] = '\0';
    }
}

void __cdecl CockosTransportNoOp()
{
}

int __cdecl CockosAudioIsNotRunning()
{
    return 0;
}

void* __cdecl CockosGetConfigVar(const char* name, int* sizeOut)
{
    if (sizeOut != nullptr)
    {
        *sizeOut = 0;
    }

    if (name == nullptr)
    {
        return nullptr;
    }

    const std::string_view configName(name);
    if (configName == "__numcpu")
    {
        if (sizeOut != nullptr)
        {
            *sizeOut = sizeof(g_reaperConfigNumCpu);
        }

        return &g_reaperConfigNumCpu;
    }

    if (configName == "__fx_loadstate_ctx")
    {
        if (sizeOut != nullptr)
        {
            *sizeOut = sizeof(g_reaperFxLoadStateContext);
        }

        return &g_reaperFxLoadStateContext;
    }

    return nullptr;
}

const char* __cdecl CockosGetIniFile()
{
    return g_emptyReaperIniPath;
}

int __cdecl CockosPluginRegister(const char*, void*)
{
    return 0;
}

intptr_t ResolveCockosHostFunction(const char* functionName)
{
    if (functionName == nullptr)
    {
        return 0;
    }

    const std::string_view name(functionName);
    if (name == "GetPlayPosition" || name == "GetPlayPosition2" || name == "GetCursorPosition")
    {
        return reinterpret_cast<intptr_t>(&CockosGetZeroTime);
    }

    if (name == "GetPlayState")
    {
        return reinterpret_cast<intptr_t>(&CockosGetStoppedPlayState);
    }

    if (name == "GetPlayStateEx")
    {
        return reinterpret_cast<intptr_t>(&CockosGetStoppedPlayStateEx);
    }

    if (name == "SetEditCurPos")
    {
        return reinterpret_cast<intptr_t>(&CockosSetEditCursorPosition);
    }

    if (name == "GetSetRepeat")
    {
        return reinterpret_cast<intptr_t>(&CockosGetSetRepeat);
    }

    if (name == "GetProjectPath")
    {
        return reinterpret_cast<intptr_t>(&CockosGetProjectPath);
    }

    if (name == "OnPlayButton" || name == "OnStopButton" || name == "OnPauseButton")
    {
        return reinterpret_cast<intptr_t>(&CockosTransportNoOp);
    }

    if (name == "IsInRealTimeAudio" || name == "Audio_IsRunning")
    {
        return reinterpret_cast<intptr_t>(&CockosAudioIsNotRunning);
    }

    if (name == "get_config_var")
    {
        return reinterpret_cast<intptr_t>(&CockosGetConfigVar);
    }

    if (name == "get_ini_file")
    {
        return reinterpret_cast<intptr_t>(&CockosGetIniFile);
    }

    if (name == "plugin_register")
    {
        return reinterpret_cast<intptr_t>(&CockosPluginRegister);
    }

    return 0;
}

intptr_t ResolveAudioMasterCanDo(const void* ptr)
{
    if (ptr == nullptr)
    {
        return 0;
    }

    const std::string_view capability(static_cast<const char*>(ptr));
    if (capability == "sendVstEvents" ||
        capability == "sendVstMidiEvent" ||
        capability == "receiveVstEvents" ||
        capability == "receiveVstMidiEvent" ||
        capability == "sizeWindow")
    {
        return 1;
    }

    return 0;
}

std::wstring FormatAudioMasterOpcodes()
{
    if (g_audioMasterOpcodes.empty())
    {
        return L"none";
    }

    std::wstring result;
    for (size_t index = 0; index < g_audioMasterOpcodes.size(); index++)
    {
        if (index > 0)
        {
            result += L",";
        }

        result += std::to_wstring(g_audioMasterOpcodes[index]);
    }

    return result;
}

std::wstring FormatCockosFunctionRequests()
{
    if (g_cockosFunctionRequests.empty())
    {
        return L"none";
    }

    std::wstring result;
    for (size_t index = 0; index < g_cockosFunctionRequests.size(); index++)
    {
        if (index > 0)
        {
            result += L",";
        }

        result += g_cockosFunctionRequests[index];
    }

    return result;
}

intptr_t __cdecl AudioMaster(
    AEffect*,
    int32_t opcode,
    int32_t index,
    intptr_t,
    void* ptr,
    float)
{
    g_audioMasterOpcodes.push_back(opcode);

    if (opcode == audioMasterCockosExtension)
    {
        if (index == cockosGetApiFunction && ptr != nullptr)
        {
            const auto* functionName = static_cast<const char*>(ptr);
            const intptr_t resolvedFunction = ResolveCockosHostFunction(functionName);
            g_cockosFunctionRequests.push_back(
                ToWide(functionName) + (resolvedFunction != 0 ? L":ok" : L":null"));
            return resolvedFunction;
        }

        if (index == cockosGetHostContext)
        {
            return 0;
        }

        return 0;
    }

    switch (opcode)
    {
    case audioMasterAutomate:
        return 0;
    case audioMasterVersion:
        return 2400;
    case audioMasterGetSampleRate:
        return 48000;
    case audioMasterGetBlockSize:
        return 512;
    case audioMasterGetInputLatency:
    case audioMasterGetOutputLatency:
        return 0;
    case audioMasterCanDo:
        return ResolveAudioMasterCanDo(ptr);
    case audioMasterGetLanguage:
        return 1;
    default:
        return 0;
    }
}

AEffect* CallEntryPointSafely(VstPluginMainProc entryPoint, bool& raisedException)
{
    raisedException = false;

    __try
    {
        g_cockosFunctionRequests.clear();
        return entryPoint(AudioMaster);
    }
    __except (EXCEPTION_EXECUTE_HANDLER)
    {
        raisedException = true;
        return nullptr;
    }
}

intptr_t CallDispatcherSafely(
    AEffect* effect,
    int32_t opcode,
    int32_t index,
    intptr_t value,
    void* ptr,
    float opt,
    bool& raisedException)
{
    raisedException = false;

    __try
    {
        return effect->dispatcher(effect, opcode, index, value, ptr, opt);
    }
    __except (EXCEPTION_EXECUTE_HANDLER)
    {
        raisedException = true;
        return 0;
    }
}

void CallProcessReplacingSafely(
    AEffect* effect,
    float** inputs,
    float** outputs,
    int32_t sampleFrames,
    bool& raisedException)
{
    raisedException = false;

    __try
    {
        effect->processReplacing(effect, inputs, outputs, sampleFrames);
    }
    __except (EXCEPTION_EXECUTE_HANDLER)
    {
        raisedException = true;
    }
}

std::wstring ToWide(const char* value)
{
    if (value == nullptr || value[0] == '\0')
    {
        return std::wstring();
    }

    const int length = MultiByteToWideChar(CP_UTF8, 0, value, -1, nullptr, 0);
    if (length <= 1)
    {
        return std::wstring();
    }

    std::wstring result(static_cast<size_t>(length - 1), L'\0');
    MultiByteToWideChar(CP_UTF8, 0, value, -1, result.data(), length);
    return result;
}

std::wstring GetWindowsErrorMessage(DWORD errorCode)
{
    wchar_t* buffer = nullptr;
    const DWORD length = FormatMessageW(
        FORMAT_MESSAGE_ALLOCATE_BUFFER | FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
        nullptr,
        errorCode,
        0,
        reinterpret_cast<wchar_t*>(&buffer),
        0,
        nullptr);

    if (length == 0 || buffer == nullptr)
    {
        return L"Windows error " + std::to_wstring(errorCode);
    }

    std::wstring message(buffer, length);
    LocalFree(buffer);

    while (!message.empty() && (message.back() == L'\r' || message.back() == L'\n' || message.back() == L' '))
    {
        message.pop_back();
    }

    return message;
}

struct Vst2HostInstance
{
    HMODULE module = nullptr;
    AEffect* effect = nullptr;
    bool opened = false;
    bool processingConfigured = false;
    bool editorOpen = false;
    int inputChannels = 0;
    int outputChannels = 0;
    std::wstring lastError;
    std::wstring pluginName;

    ~Vst2HostInstance()
    {
        ClosePlugin();
    }

    int Fail(int code, const std::wstring& message)
    {
        lastError = message;
        return code;
    }

    void SetOk()
    {
        lastError.clear();
    }

    void ClosePlugin()
    {
        if (effect != nullptr && opened && effect->dispatcher != nullptr)
        {
            if (editorOpen)
            {
                effect->dispatcher(effect, effEditClose, 0, 0, nullptr, 0.0f);
                editorOpen = false;
            }

            effect->dispatcher(effect, effMainsChanged, 0, 0, nullptr, 0.0f);
            effect->dispatcher(effect, effClose, 0, 0, nullptr, 0.0f);
        }

        effect = nullptr;
        opened = false;
        processingConfigured = false;
        inputChannels = 0;
        outputChannels = 0;
        pluginName.clear();

        if (module != nullptr)
        {
            FreeLibrary(module);
            module = nullptr;
        }
    }
};

Vst2HostInstance* FromHandle(Vst2HostInstanceHandle host)
{
    return static_cast<Vst2HostInstance*>(host);
}

void ReadPluginStrings(Vst2HostInstance& host)
{
    if (host.effect == nullptr || host.effect->dispatcher == nullptr)
    {
        return;
    }

    char name[256] = {};
    host.effect->dispatcher(host.effect, effGetEffectName, 0, 0, name, 0.0f);
    host.pluginName = ToWide(name);

    char vendor[256] = {};
    host.effect->dispatcher(host.effect, effGetVendorString, 0, 0, vendor, 0.0f);

    char product[256] = {};
    host.effect->dispatcher(host.effect, effGetProductString, 0, 0, product, 0.0f);
}

struct Vst2ChunkInfo
{
    int32_t index = -1;
    void* data = nullptr;
    intptr_t size = 0;
};

bool TryGetChunk(Vst2HostInstance& host, Vst2ChunkInfo& chunkInfo, bool& raisedException)
{
    chunkInfo = Vst2ChunkInfo();
    raisedException = false;

    if (host.effect == nullptr || host.effect->dispatcher == nullptr)
    {
        return false;
    }

    for (int32_t index = 0; index <= 1; index++)
    {
        void* chunk = nullptr;
        const intptr_t size = CallDispatcherSafely(
            host.effect,
            effGetChunk,
            index,
            0,
            &chunk,
            0.0f,
            raisedException);

        if (raisedException)
        {
            return false;
        }

        if (size > 0 && chunk != nullptr)
        {
            chunkInfo.index = index;
            chunkInfo.data = chunk;
            chunkInfo.size = size;
            return true;
        }
    }

    return false;
}
}

extern "C" VST2HOST_API int Vst2Host_GetApiVersion()
{
    return 1;
}

extern "C" VST2HOST_API Vst2HostInstanceHandle Vst2Host_Create()
{
    auto host = std::make_unique<Vst2HostInstance>();
    return host.release();
}

extern "C" VST2HOST_API void Vst2Host_Destroy(Vst2HostInstanceHandle host)
{
    std::unique_ptr<Vst2HostInstance> nativeHost(FromHandle(host));
}

extern "C" VST2HOST_API int Vst2Host_LoadPlugin(
    Vst2HostInstanceHandle host,
    const wchar_t* pluginPath)
{
    Vst2HostInstance* nativeHost = FromHandle(host);
    if (nativeHost == nullptr || pluginPath == nullptr || pluginPath[0] == L'\0')
    {
        return kErrorInvalidArgument;
    }

    nativeHost->ClosePlugin();

    const DWORD attributes = GetFileAttributesW(pluginPath);
    if (attributes == INVALID_FILE_ATTRIBUTES || (attributes & FILE_ATTRIBUTE_DIRECTORY) != 0)
    {
        return nativeHost->Fail(kErrorFileNotFound, L"VST2 plugin DLL was not found.");
    }

    HMODULE module = LoadLibraryW(pluginPath);
    if (module == nullptr)
    {
        return nativeHost->Fail(
            kErrorLoadLibrary,
            L"Unable to load VST2 DLL: " + GetWindowsErrorMessage(GetLastError()));
    }

    auto entryPoint = reinterpret_cast<VstPluginMainProc>(GetProcAddress(module, "VSTPluginMain"));
    if (entryPoint == nullptr)
    {
        entryPoint = reinterpret_cast<VstPluginMainProc>(GetProcAddress(module, "main"));
    }

    if (entryPoint == nullptr)
    {
        FreeLibrary(module);
        return nativeHost->Fail(kErrorEntryPoint, L"VST2 DLL does not export VSTPluginMain or main.");
    }

    g_audioMasterOpcodes.clear();
    bool entryPointRaisedException = false;
    AEffect* effect = CallEntryPointSafely(entryPoint, entryPointRaisedException);
    if (entryPointRaisedException)
    {
        FreeLibrary(module);
        return nativeHost->Fail(kErrorPluginLoad, L"VST2 entrypoint raised a native exception.");
    }

    if (effect == nullptr)
    {
        FreeLibrary(module);
        return nativeHost->Fail(
            kErrorPluginLoad,
            L"VST2 entrypoint returned null AEffect. audioMaster opcodes: " +
                FormatAudioMasterOpcodes() +
                L"; Cockos requests: " +
                FormatCockosFunctionRequests());
    }

    if (effect->magic != kEffectMagic)
    {
        FreeLibrary(module);
        return nativeHost->Fail(kErrorPluginLoad, L"VST2 AEffect magic is invalid.");
    }

    if (effect->dispatcher == nullptr)
    {
        FreeLibrary(module);
        return nativeHost->Fail(kErrorPluginLoad, L"VST2 plugin does not provide a dispatcher.");
    }

    nativeHost->module = module;
    nativeHost->effect = effect;

    nativeHost->effect->dispatcher(nativeHost->effect, effOpen, 0, 0, nullptr, 0.0f);
    nativeHost->opened = true;
    ReadPluginStrings(*nativeHost);
    nativeHost->SetOk();

    return kOk;
}

extern "C" VST2HOST_API int Vst2Host_SetupProcessing(
    Vst2HostInstanceHandle host,
    double sampleRate,
    int maxBlockSize,
    int inputChannels,
    int outputChannels)
{
    Vst2HostInstance* nativeHost = FromHandle(host);
    if (nativeHost == nullptr)
    {
        return kErrorInvalidArgument;
    }

    if (nativeHost->effect == nullptr)
    {
        return nativeHost->Fail(kErrorNotConfigured, L"No VST2 plugin has been loaded.");
    }

    if (sampleRate <= 0 || maxBlockSize <= 0 || inputChannels <= 0 || outputChannels <= 0)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"Invalid VST2 processing setup.");
    }

    if (nativeHost->effect->processReplacing == nullptr)
    {
        return nativeHost->Fail(kErrorUnsupported, L"VST2 plugin does not provide processReplacing.");
    }

    if (nativeHost->effect->numInputs < inputChannels || nativeHost->effect->numOutputs < outputChannels)
    {
        return nativeHost->Fail(
            kErrorUnsupported,
            L"VST2 plugin channel layout is unsupported.");
    }

    nativeHost->inputChannels = inputChannels;
    nativeHost->outputChannels = outputChannels;
    nativeHost->processingConfigured = true;

    nativeHost->effect->dispatcher(nativeHost->effect, effSetSampleRate, 0, 0, nullptr, static_cast<float>(sampleRate));
    nativeHost->effect->dispatcher(nativeHost->effect, effSetBlockSize, 0, maxBlockSize, nullptr, 0.0f);
    nativeHost->effect->dispatcher(nativeHost->effect, effMainsChanged, 0, 1, nullptr, 0.0f);
    nativeHost->SetOk();

    return kOk;
}

extern "C" VST2HOST_API int Vst2Host_ProcessFloat32(
    Vst2HostInstanceHandle host,
    const float* inputInterleaved,
    float* outputInterleaved,
    int frameCount)
{
    Vst2HostInstance* nativeHost = FromHandle(host);
    if (nativeHost == nullptr || inputInterleaved == nullptr || outputInterleaved == nullptr || frameCount < 0)
    {
        return kErrorInvalidArgument;
    }

    if (!nativeHost->processingConfigured)
    {
        return nativeHost->Fail(kErrorNotConfigured, L"VST2 processing has not been configured.");
    }

    if (frameCount == 0)
    {
        nativeHost->SetOk();
        return kOk;
    }

    if (nativeHost->effect == nullptr || nativeHost->effect->processReplacing == nullptr)
    {
        return nativeHost->Fail(kErrorNotConfigured, L"VST2 processReplacing is not available.");
    }

    const int pluginInputChannels = nativeHost->inputChannels > nativeHost->effect->numInputs
        ? nativeHost->inputChannels
        : nativeHost->effect->numInputs;
    const int pluginOutputChannels = nativeHost->outputChannels > nativeHost->effect->numOutputs
        ? nativeHost->outputChannels
        : nativeHost->effect->numOutputs;
    std::vector<std::vector<float>> inputChannels(
        static_cast<size_t>(pluginInputChannels),
        std::vector<float>(static_cast<size_t>(frameCount), 0.0f));
    std::vector<std::vector<float>> outputChannels(
        static_cast<size_t>(pluginOutputChannels),
        std::vector<float>(static_cast<size_t>(frameCount), 0.0f));
    std::vector<float*> inputPointers(static_cast<size_t>(pluginInputChannels), nullptr);
    std::vector<float*> outputPointers(static_cast<size_t>(pluginOutputChannels), nullptr);

    for (int channel = 0; channel < pluginInputChannels; channel++)
    {
        inputPointers[static_cast<size_t>(channel)] = inputChannels[static_cast<size_t>(channel)].data();
    }

    for (int channel = 0; channel < pluginOutputChannels; channel++)
    {
        outputPointers[static_cast<size_t>(channel)] = outputChannels[static_cast<size_t>(channel)].data();
    }

    for (int frame = 0; frame < frameCount; frame++)
    {
        const int inputFrameOffset = frame * nativeHost->inputChannels;
        for (int channel = 0; channel < nativeHost->inputChannels; channel++)
        {
            inputChannels[static_cast<size_t>(channel)][static_cast<size_t>(frame)] =
                inputInterleaved[inputFrameOffset + channel];
        }
    }

    bool processRaisedException = false;
    CallProcessReplacingSafely(
        nativeHost->effect,
        inputPointers.data(),
        outputPointers.data(),
        frameCount,
        processRaisedException);

    if (processRaisedException)
    {
        return nativeHost->Fail(kErrorPluginLoad, L"VST2 processReplacing raised a native exception.");
    }

    for (int frame = 0; frame < frameCount; frame++)
    {
        const int outputFrameOffset = frame * nativeHost->outputChannels;
        for (int channel = 0; channel < nativeHost->outputChannels; channel++)
        {
            outputInterleaved[outputFrameOffset + channel] =
                outputChannels[static_cast<size_t>(channel)][static_cast<size_t>(frame)];
        }
    }

    nativeHost->SetOk();

    return kOk;
}

extern "C" VST2HOST_API int Vst2Host_SaveState(
    Vst2HostInstanceHandle host,
    void* buffer,
    int bufferSize)
{
    Vst2HostInstance* nativeHost = FromHandle(host);
    if (nativeHost == nullptr)
    {
        return kErrorInvalidArgument;
    }

    if (bufferSize < 0)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"State buffer size must not be negative.");
    }

    if (nativeHost->effect == nullptr || !nativeHost->opened)
    {
        return nativeHost->Fail(kErrorNotConfigured, L"No VST2 plugin has been loaded.");
    }

    bool dispatcherRaisedException = false;
    Vst2ChunkInfo chunkInfo;
    if (TryGetChunk(*nativeHost, chunkInfo, dispatcherRaisedException))
    {
        const size_t totalSize = 12 + static_cast<size_t>(chunkInfo.size);
        if (buffer == nullptr)
        {
            nativeHost->SetOk();
            return static_cast<int>(totalSize);
        }

        if (static_cast<size_t>(bufferSize) < totalSize)
        {
            return nativeHost->Fail(kErrorInvalidArgument, L"State buffer is too small.");
        }

        char* cursor = static_cast<char*>(buffer);
        const uint32_t magic = kChunkStateBlobMagic;
        const uint32_t version = kStateBlobVersion;
        const uint32_t chunkSize = static_cast<uint32_t>(chunkInfo.size);
        memcpy(cursor, &magic, sizeof(magic));
        cursor += sizeof(magic);
        memcpy(cursor, &version, sizeof(version));
        cursor += sizeof(version);
        memcpy(cursor, &chunkSize, sizeof(chunkSize));
        cursor += sizeof(chunkSize);
        memcpy(cursor, chunkInfo.data, static_cast<size_t>(chunkInfo.size));

        nativeHost->SetOk();
        return static_cast<int>(totalSize);
    }

    if (dispatcherRaisedException)
    {
        return nativeHost->Fail(kErrorPluginLoad, L"VST2 chunk query raised a native exception.");
    }

    // No chunk support: fall back to a plain parameter value dump so the
    // plugin still restores its sound from a saved profile.
    if (nativeHost->effect->numParams <= 0 ||
        nativeHost->effect->getParameter == nullptr ||
        nativeHost->effect->setParameter == nullptr)
    {
        nativeHost->SetOk();
        return 0;
    }

    const uint32_t paramCount = static_cast<uint32_t>(nativeHost->effect->numParams);
    const size_t totalSize = 12 + static_cast<size_t>(paramCount) * sizeof(float);
    if (buffer == nullptr)
    {
        nativeHost->SetOk();
        return static_cast<int>(totalSize);
    }

    if (static_cast<size_t>(bufferSize) < totalSize)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"State buffer is too small.");
    }

    char* cursor = static_cast<char*>(buffer);
    const uint32_t magic = kParamStateBlobMagic;
    const uint32_t version = kStateBlobVersion;
    memcpy(cursor, &magic, sizeof(magic));
    cursor += sizeof(magic);
    memcpy(cursor, &version, sizeof(version));
    cursor += sizeof(version);
    memcpy(cursor, &paramCount, sizeof(paramCount));
    cursor += sizeof(paramCount);

    for (uint32_t index = 0; index < paramCount; index++)
    {
        const float value = nativeHost->effect->getParameter(
            nativeHost->effect,
            static_cast<int32_t>(index));
        memcpy(cursor, &value, sizeof(value));
        cursor += sizeof(value);
    }

    nativeHost->SetOk();
    return static_cast<int>(totalSize);
}

extern "C" VST2HOST_API int Vst2Host_LoadState(
    Vst2HostInstanceHandle host,
    const void* buffer,
    int bufferSize)
{
    Vst2HostInstance* nativeHost = FromHandle(host);
    if (nativeHost == nullptr)
    {
        return kErrorInvalidArgument;
    }

    if (buffer == nullptr || bufferSize <= 0)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"State buffer is required.");
    }

    if (nativeHost->effect == nullptr || !nativeHost->opened)
    {
        return nativeHost->Fail(kErrorNotConfigured, L"No VST2 plugin has been loaded.");
    }

    if (static_cast<size_t>(bufferSize) < 12)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"State buffer is too small.");
    }

    const char* cursor = static_cast<const char*>(buffer);
    uint32_t magic = 0;
    uint32_t version = 0;
    uint32_t payloadSize = 0;
    memcpy(&magic, cursor, sizeof(magic));
    cursor += sizeof(magic);
    memcpy(&version, cursor, sizeof(version));
    cursor += sizeof(version);
    memcpy(&payloadSize, cursor, sizeof(payloadSize));
    cursor += sizeof(payloadSize);

    if (version != kStateBlobVersion)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"State buffer version is not supported.");
    }

    if (magic == kParamStateBlobMagic || magic == kLegacyParamStateBlobMagic)
    {
        if (static_cast<size_t>(bufferSize) < 12 + static_cast<size_t>(payloadSize) * sizeof(float) ||
            payloadSize == 0)
        {
            return nativeHost->Fail(kErrorInvalidArgument, L"State buffer is truncated.");
        }

        if (nativeHost->effect->setParameter == nullptr)
        {
            return nativeHost->Fail(kErrorUnsupported, L"VST2 plugin does not accept parameter changes.");
        }

        const int32_t availableParams = nativeHost->effect->numParams;
        for (uint32_t index = 0; index < payloadSize; index++)
        {
            float value = 0.0f;
            memcpy(&value, cursor, sizeof(value));
            cursor += sizeof(value);

            if (static_cast<int32_t>(index) < availableParams)
            {
                nativeHost->effect->setParameter(
                    nativeHost->effect,
                    static_cast<int32_t>(index),
                    value);
            }
        }

        nativeHost->SetOk();
        return kOk;
    }

    if (magic != kChunkStateBlobMagic && magic != kLegacyChunkStateBlobMagic)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"State buffer is not a recognized VST-Applier state blob.");
    }

    if (static_cast<size_t>(bufferSize) < 12 + static_cast<size_t>(payloadSize))
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"State buffer is truncated.");
    }

    bool dispatcherRaisedException = false;
    Vst2ChunkInfo chunkInfo;
    if (!TryGetChunk(*nativeHost, chunkInfo, dispatcherRaisedException))
    {
        if (dispatcherRaisedException)
        {
            return nativeHost->Fail(kErrorPluginLoad, L"VST2 chunk query raised a native exception.");
        }

        return nativeHost->Fail(kErrorUnsupported, L"VST2 plugin does not support state chunks.");
    }

    bool setChunkRaisedException = false;
    CallDispatcherSafely(
        nativeHost->effect,
        effSetChunk,
        chunkInfo.index,
        static_cast<intptr_t>(payloadSize),
        const_cast<char*>(cursor),
        0.0f,
        setChunkRaisedException);

    if (setChunkRaisedException)
    {
        return nativeHost->Fail(kErrorPluginLoad, L"VST2 effSetChunk raised a native exception.");
    }

    nativeHost->SetOk();
    return kOk;
}

extern "C" VST2HOST_API int Vst2Host_OpenEditor(
    Vst2HostInstanceHandle host,
    void* parentHwnd)
{
    Vst2HostInstance* nativeHost = FromHandle(host);
    if (nativeHost == nullptr)
    {
        return kErrorInvalidArgument;
    }

    if (nativeHost->effect == nullptr)
    {
        return nativeHost->Fail(kErrorNotConfigured, L"No VST2 plugin has been loaded.");
    }

    if ((nativeHost->effect->flags & effFlagsHasEditor) == 0)
    {
        return nativeHost->Fail(kErrorUnsupported, L"VST2 plugin does not report an editor.");
    }

    if (parentHwnd == nullptr)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"VST2 editor parent window handle is null.");
    }

    nativeHost->effect->dispatcher(nativeHost->effect, effEditOpen, 0, 0, parentHwnd, 0.0f);
    nativeHost->editorOpen = true;
    nativeHost->SetOk();

    return kOk;
}

extern "C" VST2HOST_API int Vst2Host_GetEditorSize(
    Vst2HostInstanceHandle host,
    int* width,
    int* height)
{
    Vst2HostInstance* nativeHost = FromHandle(host);
    if (nativeHost == nullptr || width == nullptr || height == nullptr)
    {
        return kErrorInvalidArgument;
    }

    *width = 0;
    *height = 0;

    if (nativeHost->effect == nullptr || nativeHost->effect->dispatcher == nullptr)
    {
        return nativeHost->Fail(kErrorNotConfigured, L"No VST2 plugin has been loaded.");
    }

    ERect* editorRect = nullptr;
    bool dispatcherRaisedException = false;
    CallDispatcherSafely(
        nativeHost->effect,
        effEditGetRect,
        0,
        0,
        &editorRect,
        0.0f,
        dispatcherRaisedException);

    if (dispatcherRaisedException)
    {
        return nativeHost->Fail(kErrorPluginLoad, L"VST2 effEditGetRect raised a native exception.");
    }

    if (editorRect == nullptr)
    {
        return nativeHost->Fail(kErrorUnsupported, L"VST2 editor did not provide a size.");
    }

    const int editorWidth = editorRect->right - editorRect->left;
    const int editorHeight = editorRect->bottom - editorRect->top;
    if (editorWidth <= 0 || editorHeight <= 0)
    {
        return nativeHost->Fail(kErrorUnsupported, L"VST2 editor size is invalid.");
    }

    *width = editorWidth;
    *height = editorHeight;
    nativeHost->SetOk();

    return kOk;
}

extern "C" VST2HOST_API void Vst2Host_EditorIdle(Vst2HostInstanceHandle host)
{
    Vst2HostInstance* nativeHost = FromHandle(host);
    if (nativeHost == nullptr ||
        nativeHost->effect == nullptr ||
        nativeHost->effect->dispatcher == nullptr ||
        !nativeHost->editorOpen)
    {
        return;
    }

    bool dispatcherRaisedException = false;
    CallDispatcherSafely(
        nativeHost->effect,
        effEditIdle,
        0,
        0,
        nullptr,
        0.0f,
        dispatcherRaisedException);
}

extern "C" VST2HOST_API void Vst2Host_CloseEditor(Vst2HostInstanceHandle host)
{
    Vst2HostInstance* nativeHost = FromHandle(host);
    if (nativeHost == nullptr || nativeHost->effect == nullptr || !nativeHost->editorOpen)
    {
        return;
    }

    nativeHost->effect->dispatcher(nativeHost->effect, effEditClose, 0, 0, nullptr, 0.0f);
    nativeHost->editorOpen = false;
}

extern "C" VST2HOST_API const wchar_t* Vst2Host_GetLastError(Vst2HostInstanceHandle host)
{
    const Vst2HostInstance* nativeHost = FromHandle(host);
    if (nativeHost == nullptr)
    {
        return L"Invalid VST2 host handle.";
    }

    return nativeHost->lastError.c_str();
}
