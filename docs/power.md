# Power: CPU and memory of the GPU denoise chain

Measured on the Windows box the app ships to (Ryzen 9 + Radeon RX 9070 XT,
24 logical cores, VB-CABLE), with the app's own test seams only:

* `VA_DATA_DIR` / `VA_INSTANCE_NAME` — an isolated probe instance that runs
  beside the installed app (own mutex/event, own session.json);
* `VA_FAKE_CAPTURE=speech` — in-process synthetic capture (440 Hz tone when
  set to `1`), so no microphone is needed;
* `ADG_TRACE=1` — the GPU plugin prints per-block `den_us` to stderr.

CPU is the probe process's CPU time divided by wall time (100% = one core,
median of three 10 s samples). Memory is `WorkingSet64` / `PrivateMemorySize64`.
Run-to-run spread on this machine is about +-2 points of CPU, so treat small
differences as noise.

## States (probe, 48 kHz, 512-sample blocks)

Chain: `Applier Noise Suppression GPU` -> `Applier Gate` -> `Applier Compressor`.
"(without GPU)" drops the first slot. States (a)-(d) render to a real speaker
endpoint (the 16-channel cable input cannot be opened on this box:
AUDCLNT_E_DEVICE_IN_USE). The monitor probe (below) has to route into
`CABLE Input` - the monitor refuses to share an endpoint with the route output
- and runs for ~20 s with the chain's gate in place.

| state (median of 3 x 10 s) | v0.4.10 CPU / working / private / threads | this change |
|---|---|---|
| (a) GPU chain, window visible | 26.8% / 224 MB / 364 MB / 78 | 26.9% / 237 MB / 365 MB / 78 |
| (b) GPU chain, hidden to tray | 13.3% / 215 MB / 364 MB / 78 | 13.9% / 236 MB / 362 MB / 78 |
| (c) without GPU, visible | 10.9% / 94 MB / 191 MB / 17 | 11.7% / 94 MB / 191 MB / 17 |
| (d) route stopped, visible | 1.0% / 78 MB / 188 MB / 15 | 1.9% / 78 MB / 188 MB / 15 |
| (d) route stopped, hidden to tray | - | 0.0% / 80 MB / 188 MB / 12 |

Rows (a)-(c) are unchanged within noise: the ORT session options below do
**not** move the steady-state CPU of this chain.

### Hear-yourself monitor in the tray

The monitor state was measured on its own probe (route into the cable, so the
monitor gets a real endpoint). It costs 3-8 points of a core while it runs,
and its tray total is dominated by the route either way:

| | CPU visible | CPU tray | private | threads |
|---|---|---|---|---|
| v0.4.10, monitor armed (stream keeps running in the tray) | 29.5% | 16.3-22.1% (two runs) | 365 MB | 81 |
| this change, monitor paused on hide | 25.4% | 18.0% | 360 MB | 80 |

The CPU difference is inside the +-3 point noise of this machine; what the
change guarantees (and what is verified in the log and in `monitor_device()`)
is that no monitor stream exists while the window is hidden:

```
monitor: armed (device 'Speakers ...')
monitor: stream active on Speakers ...
monitor: paused (tray)          <- window hidden
monitor: resumed                <- window shown again
monitor: stream active on Speakers ...
```


### Where the CPU actually goes

In-app per-block timing of the denoiser (`ADG_TRACE=1`, DirectML, 480-sample
hop): mean 1.36 ms, p50 1.33 ms, p95 3.0 ms per 10 ms block, i.e. ~13 points
of one core for the model plus ~11 points for the rest of the app (engine +
UI at 30 Hz). The plugin's `process()` CPU tracks its wall time - there is no
measurable idle burn between hops on this build.

The 109% of a core reported from the field could not be reproduced here with
the same chain: the highest steady state seen was ~35% with per-block tracing
enabled (32% with the monitor on). The probe numbers above are the honest
picture of this build on this machine; a fresh measurement on the reporting
machine would need the same chain and no per-block tracing.

## ONNX Runtime session options (crates/libdf-ort/src/backend.rs)

Kept (each measured on the plugin's real 100 Hz cadence with
`crates/libdf-ort/examples/hopload.rs` and in-app with `ADG_TRACE`):

* `session.intra_op.allow_spinning = 0`
* `session.inter_op.allow_spinning = 0`
* sequential graph execution (`with_parallel_execution(false)`)
* `session.set_denormal_as_zero` (right for audio DSP)
* `ep.dml.enable_cpu_sync_spinning = 0` - DirectML's CPU fence wait spins by
  default; at 100 calls/s that is a busy-wait paying for work already on the
  GPU. Neutral-to-slightly-better per-hop time, and no thread spins.

Rejected, with numbers (all `hopload`, 8 s runs, same machine):

| knob | per-hop mean | CPU | note |
|---|---|---|---|
| defaults (v0.4.10) | 2.2-2.7 ms | 18-24% | baseline |
| `intra_op_num_threads = 1` | 2.4 ms | +2-4 points in-app | saves 57 threads / 20 MB stacks, but the encoder's nodes then run serially |
| `enable_cpu_mem_arena = 0` + `enable_mem_pattern = 0` | 2.1 ms per hop in-app | neutral | 25% slower per hop (fresh allocations every hop) for ~20 MB RSS |
| `ep.dml.enable_graph_capture = 1` | 2.8 ms | +2 points | no win for this graph |

The arena/memory-pattern defaults are therefore kept: they are what makes the
per-hop cost ~1.3 ms instead of ~2.1 ms.

## Tray behaviour

* Hiding to the tray stops the hear-yourself monitor (`monitor: paused (tray)`)
  and showing it again re-opens the stream (`monitor: resumed`). The persisted
  MONITOR preference is untouched, and the log says paused/resumed rather than
  disabled so it cannot be mistaken for a user action.
* Open plugin editors are closed on hide (`stage: editors closed (tray)`): their
  windows, DCs and font caches have nothing to paint into, and re-opening one
  is a click.
* The route itself keeps running in the tray - that is the product. Its cost is
  what remains in row (b): the chain and the loaded ORT sessions are
  irreducible while audio flows, and nothing in this change tries to unload
  them. What is reducible is the UI: with the window hidden, the repaint loop
  parks (vendored eframe patch, v0.4.7) and the app burns 0.0-0.1% of a core
  with no route (row d-tray).

Residual, stated plainly: with the GPU chain live, tray CPU is the route's CPU
(~14% in row b on this machine), not 0.1-0.5%. That budget is met by the UI
and event loop; it cannot be met by an audio path that is actively denoising.

## Reproducing

```
# in-process benchmark of the ORT backend at the plugin's cadence
DF_ORT_DIR=<dir with onnxruntime.dll> \
  cargo run -p df-ort --release --example hopload -- 10

# per-block timing inside the app (stderr)
ADG_TRACE=1 <probe exe> ...
```
