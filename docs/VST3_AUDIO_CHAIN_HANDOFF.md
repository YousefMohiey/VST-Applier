# VST3 audio chain handoff

## Goal

Implement the next Snj Voice Changer milestone: route live audio through the selected VST3 plugin chain in order.

The existing app already has:

- working input -> output passthrough;
- VST3 scan/add/remove UI;
- real VST3 editor windows;
- native C ABI:
  - `SnjVstHost_SetupProcessing(...)`
  - `SnjVstHost_ProcessFloat32(...)`

This milestone should make the audio path:

```text
selected input device
    -> capture buffer
    -> plugin 1
    -> plugin 2
    -> plugin 3
    -> selected output device
```

## Constraints

- Do not touch any driver branch/files.
- Do not run builds or tests. The user builds manually in Visual Studio and reports results.
- Keep the existing C ABI stable if possible.
- Keep editor hosting working.
- Keep passthrough working when the plugin chain is empty or when processing setup fails.
- Prefer narrow, incremental MVP behavior over a perfect DAW-grade host.

## Native C++ processing target

Files:

- `SnjVstHostNative\SnjVstHostNative.cpp`
- `SnjVstHostNative\SnjVstHostNative.h`
- `SnjVstHostNative\SnjVstHostNative.vcxproj`
- `SnjVstHostNative\SnjVstHostNative.vcxproj.filters`

Current native state:

- `LoadPlugin` loads the VST3 module and remembers the selected audio effect class.
- `OpenEditor` creates a separate `PlugProvider`/controller/view for the UI.
- `SetupProcessing` currently only stores sample rate/block/channel info.
- `ProcessFloat32` currently copies input to output.

Implement real VST3 audio processing for one native host instance:

1. Add a processing state to `SnjVstHost`.
2. On `SetupProcessing`:
   - require a loaded plugin;
   - create or reuse a `Steinberg::Vst::PlugProvider` for processing;
   - get `IComponent` and `IAudioProcessor`;
   - configure mono or stereo only for this MVP;
   - activate the first audio input and output buses;
   - use `Steinberg::Vst::ProcessSetup` with:
     - `processMode = kRealtime`
     - `symbolicSampleSize = kSample32`
     - provided `maxBlockSize`
     - provided `sampleRate`
   - call `setupProcessing`;
   - call `component->setActive(true)`;
   - call `processor->setProcessing(true)`.
3. On `ProcessFloat32`:
   - accept interleaved float input/output from C#;
   - deinterleave into planar float channel buffers;
   - call `IAudioProcessor::process(ProcessData&)`;
   - interleave the first output bus back into `outputInterleaved`;
   - for unsupported/no-output failures, return a clear native error;
   - do not crash on zero frames.
4. On `CloseEditor`, do not tear down processing.
5. On plugin reload/destroy:
   - stop processing if active;
   - call `processor->setProcessing(false)`;
   - call `component->setActive(false)`;
   - release processing state safely.

Use SDK references:

- `third_party\vst3sdk\public.sdk\samples\vst-hosting\audiohost\source\media\audioclient.cpp`
- `third_party\vst3sdk\public.sdk\source\vst\hosting\processdata.h`
- `third_party\vst3sdk\pluginterfaces\vst\ivstaudioprocessor.h`
- `third_party\vst3sdk\pluginterfaces\vst\ivstcomponent.h`

It is acceptable to manually prepare `ProcessData` with one input bus and one output bus instead of pulling in the full `HostProcessData` helper, if that keeps the DLL simpler.

## C# chain integration target

Files:

- `SnjVoiceChanger\AudioRoutingService.cs`
- `SnjVoiceChanger\Form1.cs`
- `SnjVoiceChanger\NativeVstHost.cs`
- `SnjVoiceChanger\VstPluginChainItem.cs`
- optional small new helper under `SnjVoiceChanger\*.cs`

Current C# route:

```text
WasapiCapture.DataAvailable
    -> BufferedWaveProvider
    -> MediaFoundationResampler
    -> WasapiOut
```

Implement an MVP chain insertion:

1. Let `Start` receive the current chain in order.
2. Before capture starts, call `SetupProcessing` on each plugin host using the capture format:
   - sample rate from capture format;
   - channel count from capture format;
   - output channel count same as input channel count;
   - max block size big enough for expected capture callback chunks.
3. In `Capture_DataAvailable`:
   - convert the captured bytes to interleaved float samples;
   - pass the float buffer sequentially through every enabled chain item:
     - plugin 1 input -> temp output;
     - temp output -> plugin 2;
     - etc.;
   - convert final float buffer back to the original capture wave format;
   - add processed bytes to `_buffer`.
4. If the chain is empty, keep the existing passthrough path.
5. If setup or process throws, stop applying VST for that route and keep passthrough rather than killing the app.
6. Update the route/status label enough that the user can tell whether VST processing is active or bypassed.

Format support for MVP:

- PCM 16-bit
- PCM 24-bit
- PCM 32-bit
- IEEE float 32-bit

Use `AudioBufferLevelCalculator` style as reference for conversion.

## Manual verification expected from user

After rebuild:

1. Launch app.
2. Select real microphone and `CABLE Input`.
3. Add one CANA plugin.
4. Open its editor and set an obvious effect.
5. Click `Start`.
6. In Windows/Meet/Chrome, listen/check `CABLE Output`.
7. Add multiple plugins and confirm the sound changes cumulatively in chain order.
8. Remove all plugins and confirm plain passthrough still works.

## Non-goals

- No plugin preset persistence.
- No bypass checkbox yet.
- No reorder buttons yet.
- No MIDI/events.
- No sample-accurate parameter automation.
- No latency compensation.
- No VST3 state save/load.
