//! Linear-interpolation sample-rate converter with exact block
//! bookkeeping (mirrors `ResampleMonoInto`: keeps position/tail across
//! blocks so boundaries don't click — and, critically, never gains or
//! loses samples over time).
//!
//! Why exactness matters: the capture thread produces and the render
//! thread consumes at nominally equal rates. The old code dropped one
//! input frame per block (~2300 ppm shortfall), so the ring drained at
//! ~100 samples/s no matter what: chronic render starvation, periodic
//! zero-fill, and the resulting ~100 Hz buzz. This version accounts
//! every input frame and every output sample; long-run error is bounded
//! by ±1 sample. A small occupancy servo (±0.3%) absorbs residual
//! hardware clock drift between the mic and the cable.

/// Block-wise resample with a two-sample tail carried across calls.
///
/// State is `(position, tail0, tail1)`: `position` is the fractional
/// input-phase in `[0, 1)` and `tail0/tail1` are the last two input
/// samples, addressable as virtual indices `-2`/`-1`. Every input
/// frame is either consumed or carried — none are ever skipped.
///
/// `servo` multiplies the conversion ratio to absorb clock drift:
/// `< 1` emits slightly more per block (recovering from starvation),
/// `> 1` emits slightly fewer (relieving backpressure). Callers should
/// clamp it to about ±0.5% so pitch wobble stays inaudible.
///
/// Returns the number of output frames written.
pub fn resample_with_carry(
    state: &mut (f64, f32, f32),
    input: &[f32],
    src_rate: u32,
    output: &mut [f32],
    dst_rate: u32,
    servo: f64,
) -> usize {
    if src_rate == dst_rate {
        let n = input.len().min(output.len());
        output[..n].copy_from_slice(&input[..n]);
        return n;
    }
    // Empty blocks carry nothing (cpal never delivers these, and the
    // single-sample path below handles N == 1 losslessly).
    if input.is_empty() || output.is_empty() {
        return 0;
    }
    let ratio = src_rate as f64 / dst_rate as f64 * servo;
    if !(ratio > 0.0) || !ratio.is_finite() {
        return 0;
    }
    let (pos, t0, t1) = *state;
    let n = input.len() as i64;
    // Extended sample lookup: virtual -2/-1 hit the carried tail.
    let ext = |i: i64| -> f32 {
        if i == -2 {
            t0
        } else if i == -1 {
            t1
        } else {
            input[i as usize]
        }
    };
    let mut written = 0;
    // Emit while both taps of the pair are inside this block.
    while written < output.len() {
        let t = pos + written as f64 * ratio;
        let idx = t.floor() as i64;
        if idx < -2 || idx + 1 > n - 1 {
            break;
        }
        let frac = (t - idx as f64) as f32;
        let a = ext(idx);
        let b = ext(idx + 1);
        output[written] = a * (1.0 - frac) + b * frac;
        written += 1;
    }
    // Rebase by the FULL block: next block's input[0] continues exactly
    // after this block's input[N-1], so stream positions advance by N
    // and the long-run emitted count equals input/ratio ±1 sample.
    // The last two input frames become the new tail (duplicated when
    // N == 1); `pos` stays in [-1, ratio) so the -2 guard always holds.
    let tail0 = if n >= 2 { input[n as usize - 2] } else { input[0] };
    state.0 = pos + written as f64 * ratio - n as f64;
    state.1 = tail0;
    state.2 = input[n as usize - 1];
    written
}

/// Occupancy servo: nudge the conversion ratio to hold the ring near
/// its target fill. `level_samples` is the current ring occupancy,
/// `target_samples` the desired fill. `integral` is the controller's
/// persistent state (one f64 per route, lives in the capture scratch).
///
/// This is a PI controller on purpose: a P-only servo must sit at a
/// constant droop error to counter constant drift, and USB/VM clock
/// pairs here run ~0.05-0.25% apart — right at a ±0.3% P-only clamp,
/// which let the ring drain over ~90 s until it starved. The integral
/// term *learns* the drift and cancels it, so the ring parks at the
/// target with zero steady-state error, and the output pitch lands on
/// the true rate (the integral is exactly the drift — no net shift).
///
/// Returns a ratio multiplier clamped to ±1%: enough for any consumer
/// clock pair, ~17 cents worst-case during transients, then the
/// integral takes over and the error decays.
pub fn occupancy_servo(integral: &mut f64, level_samples: f64, target_samples: f64) -> f64 {
    if !(target_samples > 0.0) {
        return 1.0;
    }
    let err = ((target_samples - level_samples) / target_samples).clamp(-1.0, 1.0);
    // Integral gain: ~5-10 s to learn a 0.1% drift at ~100 blocks/s.
    *integral = (*integral + 0.00001 * err).clamp(-0.01, 0.01);
    (1.0 - 0.003 * err - *integral).clamp(0.99, 1.01)
}
