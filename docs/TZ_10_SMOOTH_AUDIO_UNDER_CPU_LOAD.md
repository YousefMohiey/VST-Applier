# TZ_10 - Smooth Audio Under CPU Load

## Problem

When the CPU is busy (games, streaming, builds), the microphone audio could lag:
the processing thread gets starved, capture blocks arrive late and in bursts, and
the delay that builds up in the route buffer stays there afterwards. GC pauses and
per-block allocations could also cause glitches.

## Fixes in this change

### 1. Audio thread scheduling (MMCSS "Pro Audio")

The first capture callback registers the audio processing thread with the Windows
multimedia scheduler (`AvSetMmThreadCharacteristicsW("Pro Audio")` plus
`AvSetMmThreadPriority(..., HIGH)`) and raises its thread priority to `Highest`.
This is the same mechanism audio drivers and DAWs use, so the processing thread
keeps its time slices while other work saturates the CPU.

### 2. Bounded latency budget (the important one)

The route buffer used to keep up to 1000 ms of backlog: a CPU spike pushed delay
into the buffer and, because the output drains at real time only, the delay could
stick (this is what "the mic lags" sounds like).

Now, whenever the buffered audio exceeds **300 ms**, the service drops the oldest
audio down to **150 ms** (`AudioRouteBuffer.TrimOldestMilliseconds`). A short blip
under extreme load beats a permanently delayed microphone.

### 3. No allocations in the hot path

Every processing buffer (capture conversion, mono downmix, plugin chain, resample,
route output) is now a reused member buffer that only grows when a larger block
arrives. The capture callback no longer allocates, so GC pauses cannot glitch the
audio.

### 4. GC + process priority

While routing, the app uses `GCLatencyMode.SustainedLowLatency` (restored on stop)
and runs its process at `AboveNormal` priority (also restored on stop; games run at
High and are not affected).

## Verified

- `AudioRouteBufferTest` (5 checks): the trim keeps exactly the newest audio
  (oldest dropped, stream continues in order, trim below target is a no-op).
- CPU-load test: with all cores saturated by burners, the route kept running with
  the VST chain active and the buffered latency stayed in its normal range
  (about 40-100 ms sawtooth, no backlog).
- Process-freeze test: a 2 second full process suspension did not leave any
  delay behind after resume (70 ms buffered, back to normal).

## User-facing knob

The Buffer dropdown still trades latency for resilience: if a machine still
glitches under heavy load, raising the buffer (1024/2048) makes the plugin
processing block bigger and more forgiving; lowering it (128/256) reduces
latency further.
