#include "pch.h"
#include "Vst3HostNative.h"

#include "pluginterfaces/base/funknown.h"
#include "pluginterfaces/gui/iplugview.h"
#include "pluginterfaces/vst/ivstaudioprocessor.h"
#include "pluginterfaces/vst/ivsteditcontroller.h"
#include "pluginterfaces/vst/ivstprocesscontext.h"
#include "pluginterfaces/vst/vstspeaker.h"
#include "public.sdk/source/common/memorystream.h"
#include "public.sdk/source/vst/hosting/hostclasses.h"
#include "public.sdk/source/vst/hosting/module.h"
#include "public.sdk/source/vst/hosting/parameterchanges.h"
#include "public.sdk/source/vst/hosting/plugprovider.h"

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <mutex>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

namespace
{
constexpr int kSuccess = 0;
constexpr int kErrorInvalidHost = -1;
constexpr int kErrorInvalidArgument = -2;
constexpr int kErrorFileNotFound = -3;
constexpr int kErrorUnsupported = -4;
constexpr int kErrorNotConfigured = -5;
constexpr int kErrorNotImplemented = -6;
constexpr int kErrorPluginLoad = -7;
constexpr int kErrorNoFactory = -8;
constexpr int kErrorNoAudioEffect = -9;
constexpr int kErrorNoEditor = -10;
constexpr int kErrorEditorAttach = -11;

constexpr uint32_t kStateBlobMagic = 0x41545356; // 'VSTA'
constexpr uint32_t kParamStateBlobMagic = 0x50545356; // 'VSTP'
constexpr uint32_t kLegacyStateBlobMagic = 0x534A4E53; // legacy (v1.3) state magic
constexpr uint32_t kLegacyParamStateBlobMagic = 0x514A4E53; // legacy (v1.3) param magic
constexpr uint32_t kStateBlobVersion = 1;
constexpr uint32_t kMaxStateBlobSize = 64u * 1024u * 1024u;

const wchar_t kNullHostError[] = L"Invalid host handle.";
constexpr Steinberg::uint64 kAllChannelsSilent = 0xffffffffffffffffULL;

bool SameViewRect(const Steinberg::ViewRect& left, const Steinberg::ViewRect& right)
{
    return left.left == right.left &&
        left.top == right.top &&
        left.right == right.right &&
        left.bottom == right.bottom;
}

class NativePlugFrame final : public Steinberg::IPlugFrame
{
public:
    explicit NativePlugFrame(HWND parentHwnd)
        : parentHwnd_(parentHwnd)
    {
    }

    void SetPlugView(Steinberg::IPlugView* plugView)
    {
        plugView_ = plugView;
    }

    void ResizeParentToViewSize(const Steinberg::ViewRect& viewSize) const
    {
        if (!IsWindow(parentHwnd_))
        {
            return;
        }

        const int width = std::max<int>(1, viewSize.right - viewSize.left);
        const int height = std::max<int>(1, viewSize.bottom - viewSize.top);
        const DWORD style = static_cast<DWORD>(GetWindowLongPtrW(parentHwnd_, GWL_STYLE));

        if ((style & WS_CHILD) == WS_CHILD)
        {
            SetWindowPos(
                parentHwnd_,
                nullptr,
                0,
                0,
                width,
                height,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
            return;
        }

        RECT windowRect{ 0, 0, width, height };
        const DWORD extendedStyle = static_cast<DWORD>(GetWindowLongPtrW(parentHwnd_, GWL_EXSTYLE));
        const BOOL hasMenu = GetMenu(parentHwnd_) != nullptr;

        if (!AdjustWindowRectEx(&windowRect, style, hasMenu, extendedStyle))
        {
            windowRect = RECT{ 0, 0, width, height };
        }

        SetWindowPos(
            parentHwnd_,
            nullptr,
            0,
            0,
            windowRect.right - windowRect.left,
            windowRect.bottom - windowRect.top,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    }

    Steinberg::tresult PLUGIN_API resizeView(
        Steinberg::IPlugView* view,
        Steinberg::ViewRect* newSize) override
    {
        if (view == nullptr || newSize == nullptr || view != plugView_)
        {
            return Steinberg::kInvalidArgument;
        }

        if (resizeViewInProgress_)
        {
            return Steinberg::kResultFalse;
        }

        Steinberg::ViewRect currentSize{};
        if (view->getSize(&currentSize) == Steinberg::kResultTrue && SameViewRect(currentSize, *newSize))
        {
            return Steinberg::kResultTrue;
        }

        resizeViewInProgress_ = true;
        ResizeParentToViewSize(*newSize);
        const Steinberg::tresult result = view->onSize(newSize);
        resizeViewInProgress_ = false;
        return result == Steinberg::kResultTrue ? Steinberg::kResultTrue : result;
    }

    Steinberg::tresult PLUGIN_API queryInterface(const Steinberg::TUID iid, void** obj) override
    {
        if (obj == nullptr)
        {
            return Steinberg::kInvalidArgument;
        }

        *obj = nullptr;
        if (Steinberg::FUnknownPrivate::iidEqual(iid, Steinberg::IPlugFrame::iid) ||
            Steinberg::FUnknownPrivate::iidEqual(iid, Steinberg::FUnknown::iid))
        {
            *obj = static_cast<Steinberg::IPlugFrame*>(this);
            addRef();
            return Steinberg::kResultTrue;
        }

        return Steinberg::kNoInterface;
    }

    Steinberg::uint32 PLUGIN_API addRef() override
    {
        return 1000;
    }

    Steinberg::uint32 PLUGIN_API release() override
    {
        return 1000;
    }

private:
    HWND parentHwnd_ = nullptr;
    Steinberg::IPlugView* plugView_ = nullptr;
    bool resizeViewInProgress_ = false;
};

class HostComponentHandler final : public Steinberg::Vst::IComponentHandler
{
public:
    HostComponentHandler()
        : parameterTransfer_(1000)
    {
    }

    Steinberg::tresult PLUGIN_API beginEdit(Steinberg::Vst::ParamID /*id*/) override
    {
        return Steinberg::kResultTrue;
    }

    Steinberg::tresult PLUGIN_API performEdit(
        Steinberg::Vst::ParamID id,
        Steinberg::Vst::ParamValue valueNormalized) override
    {
        std::lock_guard<std::mutex> lock(parameterTransferMutex_);
        parameterTransfer_.addChange(id, valueNormalized, 0);
        return Steinberg::kResultTrue;
    }

    Steinberg::tresult PLUGIN_API endEdit(Steinberg::Vst::ParamID /*id*/) override
    {
        return Steinberg::kResultTrue;
    }

    Steinberg::tresult PLUGIN_API restartComponent(Steinberg::int32 /*flags*/) override
    {
        return Steinberg::kResultTrue;
    }

    void TransferChangesTo(Steinberg::Vst::ParameterChanges& destination)
    {
        std::lock_guard<std::mutex> lock(parameterTransferMutex_);
        parameterTransfer_.transferChangesTo(destination);
    }

    void ClearPendingChanges()
    {
        std::lock_guard<std::mutex> lock(parameterTransferMutex_);
        parameterTransfer_.removeChanges();
    }

    Steinberg::tresult PLUGIN_API queryInterface(const Steinberg::TUID iid, void** obj) override
    {
        if (obj == nullptr)
        {
            return Steinberg::kInvalidArgument;
        }

        *obj = nullptr;
        if (Steinberg::FUnknownPrivate::iidEqual(iid, Steinberg::Vst::IComponentHandler::iid) ||
            Steinberg::FUnknownPrivate::iidEqual(iid, Steinberg::FUnknown::iid))
        {
            *obj = static_cast<Steinberg::Vst::IComponentHandler*>(this);
            addRef();
            return Steinberg::kResultTrue;
        }

        return Steinberg::kNoInterface;
    }

    Steinberg::uint32 PLUGIN_API addRef() override
    {
        return 1000;
    }

    Steinberg::uint32 PLUGIN_API release() override
    {
        return 1000;
    }

private:
    std::mutex parameterTransferMutex_;
    Steinberg::Vst::ParameterChangeTransfer parameterTransfer_;
};

struct EditorState
{
    Steinberg::IPtr<Steinberg::Vst::PlugProvider> plugProvider;
    Steinberg::IPtr<Steinberg::Vst::IEditController> controller;
    Steinberg::IPtr<Steinberg::IPlugView> plugView;
    std::unique_ptr<NativePlugFrame> plugFrame;
    bool attached = false;
};

void CloseEditorState(EditorState& editor)
{
    if (editor.plugView)
    {
        editor.plugView->setFrame(nullptr);

        if (editor.attached)
        {
            editor.plugView->removed();
            editor.attached = false;
        }
    }

    editor.plugView = nullptr;
    editor.plugFrame.reset();
    editor.controller = nullptr;
    editor.plugProvider = nullptr;
}

void CloseEditorState(std::unique_ptr<EditorState>& editor)
{
    if (editor)
    {
        CloseEditorState(*editor);
        editor.reset();
    }
}

struct ProcessingState
{
    Steinberg::IPtr<Steinberg::Vst::PlugProvider> plugProvider;
    Steinberg::IPtr<Steinberg::Vst::IComponent> component;
    Steinberg::IPtr<Steinberg::Vst::IAudioProcessor> processor;
    Steinberg::Vst::ProcessContext processContext{};
    Steinberg::Vst::ProcessData processData{};
    Steinberg::Vst::ParameterChanges inputParameterChanges{1000};
    std::array<Steinberg::Vst::AudioBusBuffers, 1> inputBuses{};
    std::array<Steinberg::Vst::AudioBusBuffers, 1> outputBuses{};
    std::array<Steinberg::Vst::Sample32*, 2> inputChannelPointers{};
    std::array<Steinberg::Vst::Sample32*, 2> outputChannelPointers{};
    std::array<std::vector<Steinberg::Vst::Sample32>, 2> inputBuffers;
    std::array<std::vector<Steinberg::Vst::Sample32>, 2> outputBuffers;
    double sampleRate = 0.0;
    int maxBlockSize = 0;
    int inputChannels = 0;
    int outputChannels = 0;
    bool active = false;
    bool processing = false;
};

void StopProcessingState(ProcessingState& processing)
{
    if (processing.processor && processing.processing)
    {
        processing.processor->setProcessing(false);
    }

    processing.processing = false;

    if (processing.component && processing.active)
    {
        processing.component->setActive(false);
    }

    processing.active = false;
}

void CloseProcessingState(std::unique_ptr<ProcessingState>& processing)
{
    if (!processing)
    {
        return;
    }

    StopProcessingState(*processing);
    processing->processor = nullptr;
    processing->component = nullptr;
    processing->plugProvider = nullptr;
    processing.reset();
}

struct Vst3HostInstance
{
    std::wstring pluginPath;
    std::wstring lastError;
    double sampleRate = 0.0;
    int maxBlockSize = 0;
    int inputChannels = 0;
    int outputChannels = 0;
    bool processingConfigured = false;
    VST3::Hosting::Module::Ptr module;
    VST3::Hosting::ClassInfo selectedEffectClass;
    bool hasSelectedEffectClass = false;
    std::string selectedEffectClassId;
    std::string selectedEffectClassName;
    Steinberg::Vst::HostApplication pluginContext;
    HostComponentHandler componentHandler;
    Steinberg::IPtr<Steinberg::Vst::PlugProvider> plugProvider;
    std::unique_ptr<EditorState> editor;
    std::unique_ptr<ProcessingState> processing;

    ~Vst3HostInstance()
    {
        ClearHostParameterState();
        CloseEditorState(editor);
        CloseProcessingState(processing);
        plugProvider = nullptr;
        ClearPluginContextIfOwned();
    }

    void ClearError()
    {
        lastError.clear();
    }

    int Fail(int code, const wchar_t* message)
    {
        lastError = message != nullptr ? message : L"Unknown error.";
        return code;
    }

    int Fail(int code, const std::wstring& message)
    {
        lastError = message.empty() ? L"Unknown error." : message;
        return code;
    }

    void ResetPlugin()
    {
        ClearHostParameterState();
        CloseEditorState(editor);
        CloseProcessingState(processing);
        plugProvider = nullptr;
        ResetProcessingConfiguration();
        ClearPluginContextIfOwned();
        hasSelectedEffectClass = false;
        selectedEffectClassId.clear();
        selectedEffectClassName.clear();
        selectedEffectClass = VST3::Hosting::ClassInfo();
        module.reset();
        pluginPath.clear();
    }

    void ResetProcessingConfiguration()
    {
        sampleRate = 0.0;
        maxBlockSize = 0;
        inputChannels = 0;
        outputChannels = 0;
        processingConfigured = false;
    }

    void ClearHostParameterState()
    {
        componentHandler.ClearPendingChanges();
    }

    void ClearPluginContextIfOwned()
    {
        auto& contextFactory = Steinberg::Vst::PluginContextFactory::instance();
        if (contextFactory.getPluginContext() == &pluginContext)
        {
            contextFactory.setPluginContext(nullptr);
        }
    }
};

bool EnsurePluginProvider(Vst3HostInstance& host, const VST3::Hosting::PluginFactory& factory)
{
    if (host.plugProvider)
    {
        auto controller = host.plugProvider->getControllerPtr();
        if (controller)
        {
            controller->setComponentHandler(&host.componentHandler);
        }

        return true;
    }

    host.plugProvider = Steinberg::owned(new Steinberg::Vst::PlugProvider(
        factory,
        host.selectedEffectClass,
        true));

    if (!host.plugProvider || !host.plugProvider->initialize())
    {
        host.plugProvider = nullptr;
        return false;
    }

    auto controller = host.plugProvider->getControllerPtr();
    if (controller)
    {
        controller->setComponentHandler(&host.componentHandler);
    }

    return true;
}

Vst3HostInstance* FromHandle(Vst3HostInstanceHandle host)
{
    return static_cast<Vst3HostInstance*>(host);
}

bool EndsWithVst3(const std::wstring& path)
{
    constexpr wchar_t kExtension[] = L".vst3";
    constexpr size_t kExtensionLength = 5;

    if (path.length() < kExtensionLength)
    {
        return false;
    }

    const wchar_t* suffix = path.c_str() + path.length() - kExtensionLength;
    return _wcsicmp(suffix, kExtension) == 0;
}

bool PathExists(const wchar_t* path)
{
    const DWORD attributes = GetFileAttributesW(path);
    return attributes != INVALID_FILE_ATTRIBUTES;
}

std::string WideToUtf8(const std::wstring& value)
{
    if (value.empty())
    {
        return {};
    }

    const int byteCount = WideCharToMultiByte(
        CP_UTF8,
        WC_ERR_INVALID_CHARS,
        value.c_str(),
        static_cast<int>(value.length()),
        nullptr,
        0,
        nullptr,
        nullptr);

    if (byteCount <= 0)
    {
        throw std::runtime_error("Plugin path is not valid UTF-16.");
    }

    std::string result(static_cast<size_t>(byteCount), '\0');
    const int convertedByteCount = WideCharToMultiByte(
        CP_UTF8,
        WC_ERR_INVALID_CHARS,
        value.c_str(),
        static_cast<int>(value.length()),
        result.data(),
        byteCount,
        nullptr,
        nullptr);

    if (convertedByteCount != byteCount)
    {
        throw std::runtime_error("Plugin path conversion failed.");
    }

    return result;
}

std::wstring Utf8ToWide(const std::string& value)
{
    if (value.empty())
    {
        return {};
    }

    const int characterCount = MultiByteToWideChar(
        CP_UTF8,
        MB_ERR_INVALID_CHARS,
        value.c_str(),
        static_cast<int>(value.length()),
        nullptr,
        0);

    if (characterCount <= 0)
    {
        return std::wstring(value.begin(), value.end());
    }

    std::wstring result(static_cast<size_t>(characterCount), L'\0');
    const int convertedCharacterCount = MultiByteToWideChar(
        CP_UTF8,
        MB_ERR_INVALID_CHARS,
        value.c_str(),
        static_cast<int>(value.length()),
        result.data(),
        characterCount);

    if (convertedCharacterCount != characterCount)
    {
        return std::wstring(value.begin(), value.end());
    }

    return result;
}

std::wstring BuildError(const wchar_t* prefix, const std::string& detail)
{
    std::wstring result(prefix != nullptr ? prefix : L"");
    if (!detail.empty())
    {
        result += Utf8ToWide(detail);
    }
    return result;
}

bool IsSupportedChannelCount(int channelCount)
{
    return channelCount == 1 || channelCount == 2;
}

Steinberg::Vst::SpeakerArrangement GetSpeakerArrangement(int channelCount)
{
    return channelCount == 1 ?
        Steinberg::Vst::SpeakerArr::kMono :
        Steinberg::Vst::SpeakerArr::kStereo;
}

bool HasAudioBus(Steinberg::Vst::IComponent& component, Steinberg::Vst::BusDirection direction)
{
    return component.getBusCount(Steinberg::Vst::kAudio, direction) > 0;
}

bool IsAllSilent(const float* inputInterleaved, int frameCount, int channelCount)
{
    if (inputInterleaved == nullptr || frameCount <= 0 || channelCount <= 0)
    {
        return true;
    }

    const size_t sampleCount = static_cast<size_t>(frameCount) *
        static_cast<size_t>(channelCount);
    for (size_t sample = 0; sample < sampleCount; ++sample)
    {
        if (inputInterleaved[sample] != 0.0f)
        {
            return false;
        }
    }

    return true;
}

void PrepareProcessingData(
    Vst3HostInstance& host,
    ProcessingState& processing,
    const float* inputInterleaved,
    int frameCount)
{
    processing.inputParameterChanges.clearQueue();
    host.componentHandler.TransferChangesTo(processing.inputParameterChanges);

    for (int channel = 0; channel < processing.inputChannels; ++channel)
    {
        auto& channelBuffer = processing.inputBuffers[static_cast<size_t>(channel)];
        for (int frame = 0; frame < frameCount; ++frame)
        {
            channelBuffer[static_cast<size_t>(frame)] =
                inputInterleaved[static_cast<size_t>(frame) *
                    static_cast<size_t>(processing.inputChannels) +
                    static_cast<size_t>(channel)];
        }

        processing.inputChannelPointers[static_cast<size_t>(channel)] = channelBuffer.data();
    }

    for (int channel = 0; channel < processing.outputChannels; ++channel)
    {
        auto& channelBuffer = processing.outputBuffers[static_cast<size_t>(channel)];
        std::fill(
            channelBuffer.begin(),
            channelBuffer.begin() + frameCount,
            0.0f);
        processing.outputChannelPointers[static_cast<size_t>(channel)] = channelBuffer.data();
    }

    processing.inputBuses[0].numChannels = processing.inputChannels;
    processing.inputBuses[0].silenceFlags = IsAllSilent(
        inputInterleaved,
        frameCount,
        processing.inputChannels) ?
        kAllChannelsSilent :
        0;
    processing.inputBuses[0].channelBuffers32 = processing.inputChannelPointers.data();

    processing.outputBuses[0].numChannels = processing.outputChannels;
    processing.outputBuses[0].silenceFlags = 0;
    processing.outputBuses[0].channelBuffers32 = processing.outputChannelPointers.data();

    processing.processContext.sampleRate = processing.sampleRate;

    processing.processData.processMode = Steinberg::Vst::kRealtime;
    processing.processData.symbolicSampleSize = Steinberg::Vst::kSample32;
    processing.processData.numSamples = frameCount;
    processing.processData.numInputs = 1;
    processing.processData.numOutputs = 1;
    processing.processData.inputs = processing.inputBuses.data();
    processing.processData.outputs = processing.outputBuses.data();
    processing.processData.inputParameterChanges = &processing.inputParameterChanges;
    processing.processData.outputParameterChanges = nullptr;
    processing.processData.inputEvents = nullptr;
    processing.processData.outputEvents = nullptr;
    processing.processData.processContext = &processing.processContext;
}

void WriteProcessingOutput(
    const ProcessingState& processing,
    float* outputInterleaved,
    int frameCount)
{
    for (int frame = 0; frame < frameCount; ++frame)
    {
        for (int channel = 0; channel < processing.outputChannels; ++channel)
        {
            outputInterleaved[static_cast<size_t>(frame) *
                static_cast<size_t>(processing.outputChannels) +
                static_cast<size_t>(channel)] =
                processing.outputBuffers[static_cast<size_t>(channel)][static_cast<size_t>(frame)];
        }
    }
}

int SaveControllerParameterState(
    Vst3HostInstance& host,
    Steinberg::Vst::IEditController* controller,
    void* buffer,
    int bufferSize)
{
    if (controller == nullptr)
    {
        return host.Fail(
            kErrorPluginLoad,
            L"VST3 component state save failed and the plugin has no edit controller for a parameter fallback.");
    }

    const Steinberg::int32 parameterCount = controller->getParameterCount();
    if (parameterCount < 0)
    {
        return host.Fail(
            kErrorPluginLoad,
            L"VST3 component state save failed and the edit controller did not report its parameters.");
    }

    const size_t pairSize = sizeof(uint32_t) + sizeof(double);
    const size_t totalSize = 12 + static_cast<size_t>(parameterCount) * pairSize;

    if (buffer == nullptr)
    {
        host.ClearError();
        return static_cast<int>(totalSize);
    }

    if (static_cast<size_t>(bufferSize) < totalSize)
    {
        return host.Fail(kErrorInvalidArgument, L"State buffer is too small.");
    }

    char* cursor = static_cast<char*>(buffer);
    const uint32_t magic = kParamStateBlobMagic;
    const uint32_t version = kStateBlobVersion;
    const uint32_t count = static_cast<uint32_t>(parameterCount);
    memcpy(cursor, &magic, sizeof(magic));
    cursor += sizeof(magic);
    memcpy(cursor, &version, sizeof(version));
    cursor += sizeof(version);
    memcpy(cursor, &count, sizeof(count));
    cursor += sizeof(count);

    for (Steinberg::int32 index = 0; index < parameterCount; index++)
    {
        Steinberg::Vst::ParameterInfo info{};
        uint32_t parameterId = 0xFFFFFFFFu;
        double value = 0.0;

        if (controller->getParameterInfo(index, info) == Steinberg::kResultTrue)
        {
            parameterId = static_cast<uint32_t>(info.id);
            value = controller->getParamNormalized(info.id);
        }

        memcpy(cursor, &parameterId, sizeof(parameterId));
        cursor += sizeof(parameterId);
        memcpy(cursor, &value, sizeof(value));
        cursor += sizeof(value);
    }

    host.ClearError();
    return static_cast<int>(totalSize);
}
}

extern "C" VST3HOST_API int Vst3Host_GetApiVersion()
{
    return 1;
}

extern "C" VST3HOST_API Vst3HostInstanceHandle Vst3Host_Create()
{
    auto host = std::make_unique<Vst3HostInstance>();
    return host.release();
}

extern "C" VST3HOST_API void Vst3Host_Destroy(Vst3HostInstanceHandle host)
{
    delete FromHandle(host);
}

extern "C" VST3HOST_API int Vst3Host_LoadPlugin(
    Vst3HostInstanceHandle host,
    const wchar_t* pluginPath)
{
    Vst3HostInstance* nativeHost = FromHandle(host);
    if (nativeHost == nullptr)
    {
        return kErrorInvalidHost;
    }

    if (pluginPath == nullptr || pluginPath[0] == L'\0')
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"Plugin path is required.");
    }

    const std::wstring path(pluginPath);
    if (!EndsWithVst3(path))
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"Plugin path must end with .vst3.");
    }

    if (!PathExists(pluginPath))
    {
        return nativeHost->Fail(kErrorFileNotFound, L"Plugin path does not exist.");
    }

    nativeHost->ResetPlugin();

    std::string utf8Path;
    try
    {
        utf8Path = WideToUtf8(path);
    }
    catch (const std::exception& ex)
    {
        return nativeHost->Fail(kErrorInvalidArgument, BuildError(L"Invalid plugin path: ", ex.what()));
    }

    std::string sdkError;
    VST3::Hosting::Module::Ptr module;
    try
    {
        module = VST3::Hosting::Module::create(utf8Path, sdkError);
    }
    catch (const std::exception& ex)
    {
        return nativeHost->Fail(
            kErrorPluginLoad,
            BuildError(L"VST3 SDK failed to load the module: ", ex.what()));
    }

    if (!module)
    {
        if (sdkError.empty())
        {
            sdkError = "unknown SDK error";
        }

        return nativeHost->Fail(
            kErrorPluginLoad,
            BuildError(L"VST3 SDK failed to load the module: ", sdkError));
    }

    const VST3::Hosting::PluginFactory& factory = module->getFactory();
    if (!factory.get())
    {
        return nativeHost->Fail(kErrorNoFactory, L"VST3 module did not provide a plugin factory.");
    }

    VST3::Hosting::ClassInfo selectedClass;
    bool foundAudioEffect = false;
    try
    {
        for (const VST3::Hosting::ClassInfo& classInfo : factory.classInfos())
        {
            if (classInfo.category() == kVstAudioEffectClass)
            {
                selectedClass = classInfo;
                foundAudioEffect = true;
                break;
            }
        }
    }
    catch (const std::exception& ex)
    {
        return nativeHost->Fail(
            kErrorPluginLoad,
            BuildError(L"VST3 SDK failed while reading plugin classes: ", ex.what()));
    }

    if (!foundAudioEffect)
    {
        return nativeHost->Fail(kErrorNoAudioEffect, L"VST3 module does not contain an audio effect class.");
    }

    nativeHost->pluginPath = path;
    nativeHost->module = std::move(module);
    nativeHost->selectedEffectClass = selectedClass;
    nativeHost->hasSelectedEffectClass = true;
    nativeHost->selectedEffectClassId = selectedClass.ID().toString();
    nativeHost->selectedEffectClassName = selectedClass.name();
    nativeHost->ClearError();
    return kSuccess;
}

extern "C" VST3HOST_API int Vst3Host_SetupProcessing(
    Vst3HostInstanceHandle host,
    double sampleRate,
    int maxBlockSize,
    int inputChannels,
    int outputChannels)
{
    Vst3HostInstance* nativeHost = FromHandle(host);
    if (nativeHost == nullptr)
    {
        return kErrorInvalidHost;
    }

    if (!std::isfinite(sampleRate) || sampleRate <= 0.0)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"Sample rate must be greater than zero.");
    }

    if (maxBlockSize <= 0)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"Maximum block size must be greater than zero.");
    }

    if (!IsSupportedChannelCount(inputChannels) || !IsSupportedChannelCount(outputChannels))
    {
        return nativeHost->Fail(kErrorUnsupported, L"Only mono and stereo channel counts are supported.");
    }

    if (inputChannels != outputChannels && !(inputChannels == 1 && outputChannels == 2))
    {
        return nativeHost->Fail(kErrorUnsupported, L"Only matching channel counts or mono input to stereo output are supported.");
    }

    if (!nativeHost->module || !nativeHost->hasSelectedEffectClass)
    {
        return nativeHost->Fail(kErrorNotConfigured, L"No VST3 plugin has been loaded.");
    }

    CloseProcessingState(nativeHost->processing);
    nativeHost->ResetProcessingConfiguration();
    Steinberg::Vst::PluginContextFactory::instance().setPluginContext(&nativeHost->pluginContext);

    const VST3::Hosting::PluginFactory& factory = nativeHost->module->getFactory();
    if (!factory.get())
    {
        return nativeHost->Fail(kErrorNoFactory, L"VST3 module did not provide a plugin factory.");
    }

    if (!EnsurePluginProvider(*nativeHost, factory))
    {
        return nativeHost->Fail(kErrorPluginLoad, L"VST3 plugin component/controller initialization failed.");
    }

    auto processing = std::make_unique<ProcessingState>();
    processing->plugProvider = nativeHost->plugProvider;
    if (!processing->plugProvider)
    {
        CloseProcessingState(processing);
        return nativeHost->Fail(kErrorPluginLoad, L"VST3 plugin component/controller initialization failed.");
    }

    processing->component = processing->plugProvider->getComponentPtr();
    if (!processing->component)
    {
        CloseProcessingState(processing);
        return nativeHost->Fail(kErrorPluginLoad, L"VST3 plugin did not provide an audio component.");
    }

    Steinberg::FUnknownPtr<Steinberg::Vst::IAudioProcessor> processor(processing->component.get());
    processing->processor = processor;
    if (!processing->processor)
    {
        CloseProcessingState(processing);
        return nativeHost->Fail(kErrorUnsupported, L"VST3 component does not implement IAudioProcessor.");
    }

    if (processing->processor->canProcessSampleSize(Steinberg::Vst::kSample32) != Steinberg::kResultTrue)
    {
        CloseProcessingState(processing);
        return nativeHost->Fail(kErrorUnsupported, L"VST3 audio processor does not support 32-bit float processing.");
    }

    if (!HasAudioBus(*processing->component, Steinberg::Vst::kInput))
    {
        CloseProcessingState(processing);
        return nativeHost->Fail(kErrorUnsupported, L"VST3 audio processor does not provide an audio input bus.");
    }

    if (!HasAudioBus(*processing->component, Steinberg::Vst::kOutput))
    {
        CloseProcessingState(processing);
        return nativeHost->Fail(kErrorUnsupported, L"VST3 audio processor does not provide an audio output bus.");
    }

    Steinberg::Vst::SpeakerArrangement inputArrangement = GetSpeakerArrangement(inputChannels);
    Steinberg::Vst::SpeakerArrangement outputArrangement = GetSpeakerArrangement(outputChannels);
    if (processing->processor->setBusArrangements(
        &inputArrangement,
        1,
        &outputArrangement,
        1) != Steinberg::kResultTrue)
    {
        CloseProcessingState(processing);
        return nativeHost->Fail(kErrorUnsupported, L"VST3 audio processor does not support the requested mono/stereo bus arrangement.");
    }

    const Steinberg::int32 inputBusCount = processing->component->getBusCount(
        Steinberg::Vst::kAudio,
        Steinberg::Vst::kInput);
    for (Steinberg::int32 busIndex = 0; busIndex < inputBusCount; ++busIndex)
    {
        const bool activate = busIndex == 0;
        const Steinberg::tresult result = processing->component->activateBus(
            Steinberg::Vst::kAudio,
            Steinberg::Vst::kInput,
            busIndex,
            activate);
        if (activate && result != Steinberg::kResultTrue)
        {
            CloseProcessingState(processing);
            return nativeHost->Fail(kErrorUnsupported, L"VST3 audio processor input bus activation failed.");
        }
    }

    const Steinberg::int32 outputBusCount = processing->component->getBusCount(
        Steinberg::Vst::kAudio,
        Steinberg::Vst::kOutput);
    for (Steinberg::int32 busIndex = 0; busIndex < outputBusCount; ++busIndex)
    {
        const bool activate = busIndex == 0;
        const Steinberg::tresult result = processing->component->activateBus(
            Steinberg::Vst::kAudio,
            Steinberg::Vst::kOutput,
            busIndex,
            activate);
        if (activate && result != Steinberg::kResultTrue)
        {
            CloseProcessingState(processing);
            return nativeHost->Fail(kErrorUnsupported, L"VST3 audio processor output bus activation failed.");
        }
    }

    Steinberg::Vst::ProcessSetup processSetup{};
    processSetup.processMode = Steinberg::Vst::kRealtime;
    processSetup.symbolicSampleSize = Steinberg::Vst::kSample32;
    processSetup.maxSamplesPerBlock = maxBlockSize;
    processSetup.sampleRate = sampleRate;

    if (processing->processor->setupProcessing(processSetup) != Steinberg::kResultTrue)
    {
        CloseProcessingState(processing);
        return nativeHost->Fail(kErrorPluginLoad, L"VST3 audio processor setupProcessing failed.");
    }

    if (processing->component->setActive(true) != Steinberg::kResultTrue)
    {
        CloseProcessingState(processing);
        return nativeHost->Fail(kErrorPluginLoad, L"VST3 audio processor activation failed.");
    }

    processing->active = true;

    // Some VST3 plugins return kResultFalse from setProcessing(true) even after a
    // successful setupProcessing/setActive path. Treat process() itself as the
    // authoritative check, otherwise those plugins are permanently bypassed.
    processing->processor->setProcessing(true);
    processing->processing = true;
    processing->sampleRate = sampleRate;
    processing->maxBlockSize = maxBlockSize;
    processing->inputChannels = inputChannels;
    processing->outputChannels = outputChannels;
    processing->processContext = {};
    processing->processContext.sampleRate = sampleRate;
    processing->processContext.tempo = 120.0;

    for (int channel = 0; channel < inputChannels; ++channel)
    {
        processing->inputBuffers[static_cast<size_t>(channel)].resize(
            static_cast<size_t>(maxBlockSize));
    }

    for (int channel = 0; channel < outputChannels; ++channel)
    {
        processing->outputBuffers[static_cast<size_t>(channel)].resize(
            static_cast<size_t>(maxBlockSize));
    }

    nativeHost->sampleRate = sampleRate;
    nativeHost->maxBlockSize = maxBlockSize;
    nativeHost->inputChannels = inputChannels;
    nativeHost->outputChannels = outputChannels;
    nativeHost->processingConfigured = true;
    nativeHost->processing = std::move(processing);
    nativeHost->ClearError();
    return kSuccess;
}

extern "C" VST3HOST_API int Vst3Host_ProcessFloat32(
    Vst3HostInstanceHandle host,
    const float* inputInterleaved,
    float* outputInterleaved,
    int frameCount)
{
    Vst3HostInstance* nativeHost = FromHandle(host);
    if (nativeHost == nullptr)
    {
        return kErrorInvalidHost;
    }

    if (!nativeHost->processingConfigured)
    {
        return nativeHost->Fail(kErrorNotConfigured, L"Processing has not been configured.");
    }

    if (frameCount < 0)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"Frame count must not be negative.");
    }

    if (frameCount > nativeHost->maxBlockSize)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"Frame count exceeds configured maximum block size.");
    }

    if (frameCount == 0)
    {
        nativeHost->ClearError();
        return kSuccess;
    }

    if (inputInterleaved == nullptr || outputInterleaved == nullptr)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"Input and output buffers are required.");
    }

    if (!nativeHost->processing || !nativeHost->processing->processor || !nativeHost->processing->processing)
    {
        return nativeHost->Fail(kErrorNotConfigured, L"VST3 audio processor is not configured for processing.");
    }

    if (nativeHost->processing->outputChannels <= 0)
    {
        return nativeHost->Fail(kErrorUnsupported, L"VST3 audio processor has no configured output channels.");
    }

    PrepareProcessingData(*nativeHost, *nativeHost->processing, inputInterleaved, frameCount);

    if (nativeHost->processing->processor->process(nativeHost->processing->processData) != Steinberg::kResultTrue)
    {
        return nativeHost->Fail(kErrorPluginLoad, L"VST3 audio processor process failed.");
    }

    WriteProcessingOutput(*nativeHost->processing, outputInterleaved, frameCount);
    nativeHost->ClearError();
    return kSuccess;
}

extern "C" VST3HOST_API int Vst3Host_SaveState(
    Vst3HostInstanceHandle host,
    void* buffer,
    int bufferSize)
{
    Vst3HostInstance* nativeHost = FromHandle(host);
    if (nativeHost == nullptr)
    {
        return kErrorInvalidHost;
    }

    if (bufferSize < 0)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"State buffer size must not be negative.");
    }

    if (!nativeHost->module || !nativeHost->hasSelectedEffectClass)
    {
        return nativeHost->Fail(kErrorNotConfigured, L"No VST3 plugin has been loaded.");
    }

    Steinberg::Vst::PluginContextFactory::instance().setPluginContext(&nativeHost->pluginContext);

    const VST3::Hosting::PluginFactory& factory = nativeHost->module->getFactory();
    if (!factory.get())
    {
        return nativeHost->Fail(kErrorNoFactory, L"VST3 module did not provide a plugin factory.");
    }

    if (!EnsurePluginProvider(*nativeHost, factory))
    {
        return nativeHost->Fail(kErrorPluginLoad, L"VST3 plugin component/controller initialization failed.");
    }

    auto component = nativeHost->plugProvider->getComponentPtr();
    auto controller = nativeHost->plugProvider->getControllerPtr();

    Steinberg::MemoryStream componentStream;
    bool hasComponentState = false;
    if (component)
    {
        hasComponentState = component->getState(&componentStream) == Steinberg::kResultTrue;
    }

    if (!hasComponentState)
    {
        // Component state is unavailable (or the component itself is missing);
        // fall back to a controller parameter dump so the plugin settings are
        // still captured for the profile.
        return SaveControllerParameterState(*nativeHost, controller, buffer, bufferSize);
    }

    Steinberg::MemoryStream controllerStream;
    bool hasControllerState = false;
    if (controller)
    {
        if (controller->getState(&controllerStream) == Steinberg::kResultTrue)
        {
            hasControllerState = true;
        }
    }

    const Steinberg::TSize componentSizeRaw = componentStream.getSize();
    const Steinberg::TSize controllerSizeRaw = hasControllerState ? controllerStream.getSize() : 0;
    if (componentSizeRaw > kMaxStateBlobSize || controllerSizeRaw > kMaxStateBlobSize)
    {
        return nativeHost->Fail(kErrorUnsupported, L"VST3 plugin state is too large to persist.");
    }

    const uint32_t componentSize = static_cast<uint32_t>(componentSizeRaw);
    const uint32_t controllerSize = static_cast<uint32_t>(controllerSizeRaw);
    const size_t fixedSize = sizeof(uint32_t) * 4;
    const size_t totalSize = fixedSize + componentSize + controllerSize;

    if (buffer == nullptr)
    {
        nativeHost->ClearError();
        return static_cast<int>(totalSize);
    }

    if (static_cast<size_t>(bufferSize) < totalSize)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"State buffer is too small.");
    }

    char* cursor = static_cast<char*>(buffer);
    const uint32_t magic = kStateBlobMagic;
    const uint32_t version = kStateBlobVersion;
    memcpy(cursor, &magic, sizeof(magic));
    cursor += sizeof(magic);
    memcpy(cursor, &version, sizeof(version));
    cursor += sizeof(version);
    memcpy(cursor, &componentSize, sizeof(componentSize));
    cursor += sizeof(componentSize);
    if (componentSize > 0)
    {
        memcpy(cursor, componentStream.getData(), componentSize);
        cursor += componentSize;
    }

    memcpy(cursor, &controllerSize, sizeof(controllerSize));
    cursor += sizeof(controllerSize);
    if (controllerSize > 0)
    {
        memcpy(cursor, controllerStream.getData(), controllerSize);
        cursor += controllerSize;
    }

    nativeHost->ClearError();
    return static_cast<int>(totalSize);
}

extern "C" VST3HOST_API int Vst3Host_LoadState(
    Vst3HostInstanceHandle host,
    const void* buffer,
    int bufferSize)
{
    Vst3HostInstance* nativeHost = FromHandle(host);
    if (nativeHost == nullptr)
    {
        return kErrorInvalidHost;
    }

    if (buffer == nullptr || bufferSize <= 0)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"State buffer is required.");
    }

    if (!nativeHost->module || !nativeHost->hasSelectedEffectClass)
    {
        return nativeHost->Fail(kErrorNotConfigured, L"No VST3 plugin has been loaded.");
    }

    const size_t totalSize = static_cast<size_t>(bufferSize);
    if (totalSize < 12)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"State buffer is too small.");
    }

    const char* cursor = static_cast<const char*>(buffer);
    uint32_t magic = 0;
    uint32_t version = 0;
    uint32_t componentSize = 0;
    uint32_t controllerSize = 0;
    memcpy(&magic, cursor, sizeof(magic));
    cursor += sizeof(magic);
    memcpy(&version, cursor, sizeof(version));
    cursor += sizeof(version);
    memcpy(&componentSize, cursor, sizeof(componentSize));
    cursor += sizeof(componentSize);

    if (version != kStateBlobVersion)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"State buffer version is not supported.");
    }

    if (magic == kParamStateBlobMagic || magic == kLegacyParamStateBlobMagic)
    {
        const uint32_t parameterCount = componentSize;
        const size_t pairSize = sizeof(uint32_t) + sizeof(double);
        if (totalSize < 12 + static_cast<size_t>(parameterCount) * pairSize)
        {
            return nativeHost->Fail(kErrorInvalidArgument, L"State buffer is truncated.");
        }

        Steinberg::Vst::PluginContextFactory::instance().setPluginContext(&nativeHost->pluginContext);

        const VST3::Hosting::PluginFactory& factory = nativeHost->module->getFactory();
        if (!factory.get())
        {
            return nativeHost->Fail(kErrorNoFactory, L"VST3 module did not provide a plugin factory.");
        }

        if (!EnsurePluginProvider(*nativeHost, factory))
        {
            return nativeHost->Fail(kErrorPluginLoad, L"VST3 plugin component/controller initialization failed.");
        }

        auto controller = nativeHost->plugProvider->getControllerPtr();
        if (!controller)
        {
            return nativeHost->Fail(kErrorPluginLoad, L"VST3 edit controller is unavailable for a parameter restore.");
        }

        for (uint32_t index = 0; index < parameterCount; index++)
        {
            uint32_t parameterId = 0;
            double value = 0.0;
            memcpy(&parameterId, cursor, sizeof(parameterId));
            cursor += sizeof(parameterId);
            memcpy(&value, cursor, sizeof(value));
            cursor += sizeof(value);

            if (parameterId != 0xFFFFFFFFu)
            {
                const Steinberg::Vst::ParamID id = static_cast<Steinberg::Vst::ParamID>(parameterId);
                controller->setParamNormalized(id, value);
                nativeHost->componentHandler.performEdit(id, value);
            }
        }

        nativeHost->ClearError();
        return kSuccess;
    }

    if (magic != kStateBlobMagic && magic != kLegacyStateBlobMagic)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"State buffer is not a recognized VST-Applier state blob.");
    }

    const size_t fixedSize = sizeof(uint32_t) * 4;
    if (totalSize < fixedSize)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"State buffer is too small.");
    }

    if (componentSize > totalSize - 12 - sizeof(uint32_t))
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"State buffer is truncated.");
    }

    const char* componentData = cursor;
    cursor += componentSize;

    memcpy(&controllerSize, cursor, sizeof(controllerSize));
    cursor += sizeof(controllerSize);

    if (controllerSize > totalSize - 12 - componentSize - sizeof(uint32_t))
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"State buffer is truncated.");
    }

    const char* controllerData = cursor;

    Steinberg::Vst::PluginContextFactory::instance().setPluginContext(&nativeHost->pluginContext);

    const VST3::Hosting::PluginFactory& factory = nativeHost->module->getFactory();
    if (!factory.get())
    {
        return nativeHost->Fail(kErrorNoFactory, L"VST3 module did not provide a plugin factory.");
    }

    if (!EnsurePluginProvider(*nativeHost, factory))
    {
        return nativeHost->Fail(kErrorPluginLoad, L"VST3 plugin component/controller initialization failed.");
    }

    auto component = nativeHost->plugProvider->getComponentPtr();
    if (!component)
    {
        return nativeHost->Fail(kErrorPluginLoad, L"VST3 plugin did not provide an audio component.");
    }

    auto controller = nativeHost->plugProvider->getControllerPtr();

    if (componentSize > 0)
    {
        Steinberg::MemoryStream componentStream(
            const_cast<char*>(componentData),
            static_cast<Steinberg::TSize>(componentSize));
        if (component->setState(&componentStream) != Steinberg::kResultTrue)
        {
            return nativeHost->Fail(kErrorPluginLoad, L"VST3 component state restore failed.");
        }

        if (controller)
        {
            Steinberg::MemoryStream componentStateForController(
                const_cast<char*>(componentData),
                static_cast<Steinberg::TSize>(componentSize));
            controller->setComponentState(&componentStateForController);
        }
    }

    if (controller && controllerSize > 0)
    {
        Steinberg::MemoryStream controllerStream(
            const_cast<char*>(controllerData),
            static_cast<Steinberg::TSize>(controllerSize));
        if (controller->setState(&controllerStream) != Steinberg::kResultTrue)
        {
            return nativeHost->Fail(kErrorPluginLoad, L"VST3 controller state restore failed.");
        }
    }

    nativeHost->ClearError();
    return kSuccess;
}

extern "C" VST3HOST_API int Vst3Host_OpenEditor(
    Vst3HostInstanceHandle host,
    void* parentHwnd)
{
    Vst3HostInstance* nativeHost = FromHandle(host);
    if (nativeHost == nullptr)
    {
        return kErrorInvalidHost;
    }

    if (parentHwnd == nullptr)
    {
        return nativeHost->Fail(kErrorInvalidArgument, L"Parent window handle is required.");
    }

    if (!nativeHost->module || !nativeHost->hasSelectedEffectClass)
    {
        return nativeHost->Fail(kErrorNotConfigured, L"No VST3 plugin has been loaded.");
    }

    CloseEditorState(nativeHost->editor);
    Steinberg::Vst::PluginContextFactory::instance().setPluginContext(&nativeHost->pluginContext);

    const VST3::Hosting::PluginFactory& factory = nativeHost->module->getFactory();
    if (!factory.get())
    {
        return nativeHost->Fail(kErrorNoFactory, L"VST3 module did not provide a plugin factory.");
    }

    if (!EnsurePluginProvider(*nativeHost, factory))
    {
        return nativeHost->Fail(kErrorPluginLoad, L"VST3 plugin component/controller initialization failed.");
    }

    auto editor = std::make_unique<EditorState>();
    editor->plugProvider = nativeHost->plugProvider;

    editor->controller = editor->plugProvider->getControllerPtr();
    if (!editor->controller)
    {
        CloseEditorState(editor);
        return nativeHost->Fail(kErrorNoEditor, L"VST3 plugin does not provide an edit controller.");
    }

    editor->plugView = Steinberg::owned(editor->controller->createView(Steinberg::Vst::ViewType::kEditor));
    if (!editor->plugView)
    {
        CloseEditorState(editor);
        return nativeHost->Fail(kErrorNoEditor, L"VST3 edit controller does not provide an editor view.");
    }

    if (editor->plugView->isPlatformTypeSupported(Steinberg::kPlatformTypeHWND) != Steinberg::kResultTrue)
    {
        CloseEditorState(editor);
        return nativeHost->Fail(kErrorUnsupported, L"VST3 editor view does not support HWND hosting.");
    }

    editor->plugFrame = std::make_unique<NativePlugFrame>(static_cast<HWND>(parentHwnd));
    editor->plugFrame->SetPlugView(editor->plugView);

    if (editor->plugView->setFrame(editor->plugFrame.get()) != Steinberg::kResultTrue)
    {
        CloseEditorState(editor);
        return nativeHost->Fail(kErrorEditorAttach, L"VST3 editor setFrame failed.");
    }

    if (editor->plugView->attached(parentHwnd, Steinberg::kPlatformTypeHWND) != Steinberg::kResultTrue)
    {
        CloseEditorState(editor);
        return nativeHost->Fail(kErrorEditorAttach, L"VST3 editor attached(HWND) failed.");
    }

    editor->attached = true;

    Steinberg::ViewRect viewSize{};
    if (editor->plugView->getSize(&viewSize) == Steinberg::kResultTrue)
    {
        editor->plugFrame->ResizeParentToViewSize(viewSize);
        editor->plugView->onSize(&viewSize);
    }

    nativeHost->editor = std::move(editor);
    nativeHost->ClearError();
    return kSuccess;
}

extern "C" VST3HOST_API void Vst3Host_CloseEditor(Vst3HostInstanceHandle host)
{
    Vst3HostInstance* nativeHost = FromHandle(host);
    if (nativeHost != nullptr)
    {
        CloseEditorState(nativeHost->editor);
        nativeHost->ClearError();
    }
}

extern "C" VST3HOST_API const wchar_t* Vst3Host_GetLastError(Vst3HostInstanceHandle host)
{
    const Vst3HostInstance* nativeHost = FromHandle(host);
    if (nativeHost == nullptr)
    {
        return kNullHostError;
    }

    return nativeHost->lastError.c_str();
}
