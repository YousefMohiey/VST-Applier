# Snj Voice Changer - VST3 Native Host Handoff

This document is the handoff for a separate C++/VST implementation thread.
The main product remains the C# WinForms app. The C++ project is only a native
VST3 bridge.

## Current State

Solution:

- `SnjVoiceChanger.sln`
- `SnjVoiceChanger/` - C# WinForms app, .NET 9, x64, NAudio.
- `SnjVstHostNative/` - C++ x64 dynamic library project.

The C# app already has:

- input device selector;
- output device selector;
- VB-CABLE detection;
- direct passthrough from selected input to selected output;
- input and output signal meters;
- `SnjVstHostNative.dll` built beside `SnjVoiceChanger.exe`.

Important rule from the user:

- Do not run builds/tests from Codex. The user runs builds and reports results.

## Goal

Implement the native VST3 host bridge for one audio effect plugin first.

Target test plugin folder:

```text
C:\Work\codex\snj-voice-changer\common\VST\CANA-Epic-Vocals(64)
```

That folder contains both VST2 and VST3 builds. For this task use VST3:

```text
CANA Epic Bender (64).vst3
CANA Epic Tune (64).vst3
CANA Epic Vocals (64).vst3
```

Do not use the `.dll` files for this milestone. Those are VST2.

## Scope

Owned by the C++/VST thread:

- `SnjVstHostNative/*`
- optional native headers under `SnjVstHostNative/*`
- optional notes appended to this handoff doc

Avoid editing C# files unless explicitly coordinated with the main thread.

Out of scope:

- writing a custom Windows audio driver;
- VST2 hosting;
- instruments/MIDI plugins;
- multi-plugin chain;
- preset persistence;
- production crash isolation in a separate process.

## Recommended References

Use official Steinberg sources first:

- VST3 SDK repository: `https://github.com/steinbergmedia/vst3sdk`
- VST3 Developer Portal: `https://steinbergmedia.github.io/vst3_dev_portal/`
- SDK overview: `https://steinbergmedia.github.io/vst3_doc/sdk.overview.html`
- Editor host sample: `public.sdk/samples/vst-hosting/editorhost`
- Audio host sample: `public.sdk/samples/vst-hosting/audiohost`

The official SDK docs say the SDK includes VST3 hosting samples and a validator
test host. The `editorhost` sample is especially relevant for opening a plugin
editor in a host-provided window; the `audiohost` sample is relevant for loading
and processing audio through a plugin.

## Architecture

Keep the app split:

```text
SnjVoiceChanger.exe (C# WinForms)
  -> P/Invoke
    -> SnjVstHostNative.dll (C++ bridge)
      -> Steinberg VST3 SDK
        -> plugin.vst3
```

C# is responsible for:

- UI;
- plugin folder selector;
- plugin list;
- audio capture/render;
- converting app audio into float buffers for the native bridge;
- opening a WinForms editor window and passing its HWND to native code.

C++ is responsible for:

- loading VST3 modules;
- selecting an audio effect class;
- creating the component/controller;
- setting sample rate, block size, and channel layout;
- processing float audio blocks;
- attaching the plugin editor view to a host HWND;
- returning stable error codes/messages.

## C ABI Contract

Expose a plain C ABI from `SnjVstHostNative.dll`. Do not expose C++ classes to
C#.

MVP API:

```c
typedef void* SnjVstHostHandle;

extern "C" __declspec(dllexport)
int SnjVstHost_GetApiVersion();

extern "C" __declspec(dllexport)
SnjVstHostHandle SnjVstHost_Create();

extern "C" __declspec(dllexport)
void SnjVstHost_Destroy(SnjVstHostHandle host);

extern "C" __declspec(dllexport)
int SnjVstHost_LoadPlugin(
    SnjVstHostHandle host,
    const wchar_t* pluginPath);

extern "C" __declspec(dllexport)
int SnjVstHost_SetupProcessing(
    SnjVstHostHandle host,
    double sampleRate,
    int maxBlockSize,
    int inputChannels,
    int outputChannels);

extern "C" __declspec(dllexport)
int SnjVstHost_ProcessFloat32(
    SnjVstHostHandle host,
    const float* inputInterleaved,
    float* outputInterleaved,
    int frameCount);

extern "C" __declspec(dllexport)
int SnjVstHost_OpenEditor(
    SnjVstHostHandle host,
    void* parentHwnd);

extern "C" __declspec(dllexport)
void SnjVstHost_CloseEditor(SnjVstHostHandle host);

extern "C" __declspec(dllexport)
const wchar_t* SnjVstHost_GetLastError(SnjVstHostHandle host);
```

Return convention:

- `0` means success.
- negative values mean failure.
- the main thread will define exact enum names later.
- `SnjVstHost_GetLastError` must return a pointer that remains valid until the
  next native call on the same host or until destroy.

Threading rule:

- Assume all calls for one host happen on one app-controlled thread for MVP.
- Do not create background audio threads inside the native bridge for MVP.

Audio format rule:

- C# will eventually pass interleaved float32 buffers.
- C++ may convert interleaved to planar VST3 buffers internally.
- MVP channel count: mono or stereo.
- MVP should tolerate mono input -> stereo output by copying mono to both
  channels, or return a clear unsupported-layout error.

## First Milestone: Native Smoke Test

Before integrating the SDK, implement a minimal exported API:

- `SnjVstHost_GetApiVersion()` returns `1`.
- `Create` returns an opaque host object.
- `Destroy` frees it.
- `LoadPlugin` checks that the path exists and ends with `.vst3`, but does not
  load SDK yet.
- `ProcessFloat32` copies input to output unchanged.
- `GetLastError` returns useful messages.

This lets the C# side validate P/Invoke and audio-chain integration before VST3
SDK complexity enters the picture.

## Second Milestone: VST3 SDK Load Only

Add the official VST3 SDK and load one `.vst3` module.

Acceptance:

- `LoadPlugin(path)` can load one CANA `.vst3` module.
- It can identify/select an audio effect class.
- It reports a clear error if the module is not a VST3 plugin or has no audio
  effect.

No audio processing required yet in this milestone.

## Third Milestone: Process Audio

Add setup/process support.

Acceptance:

- `SetupProcessing(sampleRate, maxBlockSize, inputChannels, outputChannels)`
  succeeds for the current Snj audio pipeline.
- `ProcessFloat32(...)` calls the plugin process function.
- bypass/error fallback should copy input to output rather than output silence
  when possible.

## Fourth Milestone: Editor Window

Add editor attach.

Acceptance:

- C# creates a separate WinForms `Form`.
- C# passes `form.Handle` to `SnjVstHost_OpenEditor`.
- Native bridge opens/attaches the VST3 editor view to that HWND.
- `CloseEditor` detaches/closes safely.

## Notes for Main Thread

The main C# thread can build UI independently:

- right panel plugin folder textbox;
- Browse button;
- Scan button;
- list all `*.vst3`;
- Add selected plugin;
- Chain list;
- Editor button.

For initial scan, C# can show filenames only. Real VST3 metadata can come later
from the native bridge.

## Verification

Codex agents should not run builds in this project. The user builds manually.

Useful manual checks for the user:

1. Build solution in Visual Studio.
2. Confirm `SnjVstHostNative.dll` exists beside `SnjVoiceChanger.exe`.
3. Confirm C# can call `SnjVstHost_GetApiVersion()` after the smoke-test API is
   implemented.
4. Confirm passthrough still works without a loaded plugin.
5. Confirm loaded plugin changes the signal when enabled.
