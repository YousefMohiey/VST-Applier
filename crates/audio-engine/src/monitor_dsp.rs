//! Band-limited DSP for the "hear yourself" monitor.
//!
//! The monitor has to turn the chain's mono block (always at the route's
//! output rate) into whatever the headphone endpoint wants, and it has to
//! hold an 80 ms buffer against clock drift between two independent
//! devices. Both jobs are done here, without allocation on the audio
//! thread and without ever producing a sample-to-sample step.
//!
//! Why not the route's resampler: [`crate::resample::resample_with_carry`]
//! is a two-tap linear interpolator. At 48 k -> 44.1 k it leaves the
//! interpolation images in place (a 1 kHz tone comes back with sidebands
//! at 2.9 kHz and 4.9 kHz, measured at about -34 dB in
//! `tests::ratio_path_sidebands`), which is exactly the harsh buzz a
//! listener hears. It also rebases its phase by the whole input block, so
//! a caller that pops `ceil(frames * ratio) + 4` input frames for `frames`
//! outputs loses several frames per callback — at 48 k in / 48 k out that
//! is a 4-sample hop every 512 frames, i.e. a ~94 Hz click train.
//!
//! This module keeps the exact bookkeeping instead: input is pulled into
//! an internal carry buffer, output positions advance by the *fractional*
//! ratio, and leftovers stay in the buffer for the next call. Rate changes
//! (the drift servo) therefore become a fractional-delay nudge — smooth by
//! construction — instead of a skip.

use ringbuf::traits::{Consumer, Observer};
use ringbuf::HeapCons;

/// Coefficient phases per input sample of the sinc table.
const PHASES: usize = 512;
/// Kernel half-width in input samples (TAPS = 2 * HALF).
const HALF: usize = 32;
const TAPS: usize = HALF * 2;
/// Largest block rendered per internal pass (bounds the carry buffer).
pub(crate) const MAX_CHUNK: usize = 4096;
/// Fade length used by the emergency trim, in output samples (~1.3 ms).
const FADE: usize = 64;
/// DC blocker corner (Hz). Inaudible; removes any offset an integer
/// sample format (or a DC-biased capture) can hand us.
const DC_HZ: f64 = 5.0;

/// Occupancy the servo parks the ring at (audible latency of the monitor;
/// the user is listening, not performing).
pub(crate) const TARGET_MS: f64 = 80.0;

/// Source of chain-rate samples: the tee ring, or a test harness.
pub trait Source {
    fn occupied(&self) -> usize;
    fn pop(&mut self, dst: &mut [f32]) -> usize;
    fn skip(&mut self, n: usize) -> usize;
}

impl Source for HeapCons<f32> {
    fn occupied(&self) -> usize {
        self.occupied_len()
    }
    fn pop(&mut self, dst: &mut [f32]) -> usize {
        self.pop_slice(dst)
    }
    fn skip(&mut self, n: usize) -> usize {
        Consumer::skip(self, n)
    }
}

/// What one [`Pipe::render`] call did.
#[derive(Debug, Default, Clone, Copy)]
pub struct Report {
    /// Output frames actually synthesised (rest are silence).
    pub written: usize,
    /// True when the ring ran dry inside this block (one per gap).
    pub underrun: bool,
    /// Input frames fast-forwarded by the emergency trim.
    pub dropped: u64,
    pub peak: f32,
}

/// Modulated lowpass kernel: `sinc * Kaiser`, tabulated over distance.
struct Kernel {
    table: Vec<f32>,
}

fn bessel_i0(x: f64) -> f64 {
    // Series form; converges in a handful of terms for the betas used.
    let mut sum = 1.0f64;
    let mut term = 1.0f64;
    let q = x * x / 4.0;
    for k in 1..40 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < 1e-12 * sum {
            break;
        }
    }
    sum
}

impl Kernel {
    fn new(src_rate: u32, dst_rate: u32) -> Self {
        // Cutoff in cycles per *input* sample. Downsampling must band-limit
        // to the destination Nyquist; upsampling only has to interpolate,
        // so the cutoff sits at the source Nyquist. At unity rate an exact
        // Nyquist cutoff makes the integer-phase kernel a perfect delta,
        // so the common 48 k -> 48 k path is bit-transparent.
        let fc = if src_rate == dst_rate {
            0.5
        } else {
            0.5 * (dst_rate as f64 / src_rate as f64).min(1.0) * 0.96
        };
        let beta = 8.6; // ~-80 dB stopband
        let n = HALF * PHASES + 2;
        let mut table = vec![0.0f32; n];
        let denom = bessel_i0(beta);
        for (i, slot) in table.iter_mut().enumerate() {
            let d = i as f64 / PHASES as f64;
            let x = d / HALF as f64;
            let w = if x < 1.0 {
                bessel_i0(beta * (1.0 - x * x).sqrt()) / denom
            } else {
                0.0
            };
            let arg = std::f64::consts::PI * 2.0 * fc * d;
            let s = if d == 0.0 {
                1.0
            } else if fc == 0.5 {
                // Exact zeros at every integer distance: delta at phase 0.
                (std::f64::consts::PI * d).sin() / (std::f64::consts::PI * d)
            } else {
                arg.sin() / arg
            };
            *slot = (2.0 * fc * s * w) as f32;
        }
        Self { table }
    }

    #[inline]
    fn coef(&self, d: f32) -> f32 {
        let x = d * PHASES as f32;
        let j = x as usize;
        if j + 1 >= self.table.len() {
            return 0.0;
        }
        let f = x - j as f32;
        self.table[j] * (1.0 - f) + self.table[j + 1] * f
    }

    /// Normalised interpolation at `pos` (fractional index into `buf`).
    #[inline]
    fn at(&self, buf: &[f32], pos: f64) -> f32 {
        let ip = pos.floor();
        let frac = (pos - ip) as f32;
        if frac == 0.0 {
            return buf[ip as usize];
        }
        let base = ip as isize - HALF as isize + 1;
        let mut d = frac + (HALF - 1) as f32;
        let mut acc = 0.0f32;
        let mut sum = 0.0f32;
        for k in 0..TAPS {
            let c = self.coef(if d < 0.0 { -d } else { d });
            acc += buf[(base + k as isize) as usize] * c;
            sum += c;
            d -= 1.0;
        }
        if sum > 0.0 { acc / sum } else { 0.0 }
    }
}

/// Streaming monitor converter: pull -> interpolate -> servo -> DC block.
pub struct Pipe {
    src_rate: u32,
    dst_rate: u32,
    base_ratio: f64,
    kernel: Kernel,
    ring_cap: usize,
    target: f64,
    /// Input carry buffer (already popped from the ring, not yet consumed).
    buf: Vec<f32>,
    len: usize,
    /// Last ratio + input need (diagnostics/tests only).
    last_ratio: f64,
    last_need: usize,
    /// Fractional read position into `buf`; always >= HALF - 1.
    pos: f64,
    /// Smoothed ring occupancy (one-pole, ~0.5 s) that steers the servo.
    occ_avg: f64,
    /// Extra read-speed offset (0..3%) used to walk excess occupancy down
    /// without dropping samples.
    catchup: f64,
    armed: bool,
    /// True once real audio has played: switches the re-prime threshold
    /// from the full pre-roll to a short refill after a gap.
    ever: bool,
    skip_pending: usize,
    gain: f32,
    dc_x1: f32,
    dc_y1: f32,
    dc_r: f32,
}

impl Pipe {
    pub fn new(src_rate: u32, dst_rate: u32, ring_cap: usize) -> Self {
        let src_rate = src_rate.max(1);
        let dst_rate = dst_rate.max(1);
        let base_ratio = src_rate as f64 / dst_rate as f64;
        let cap = (MAX_CHUNK as f64 * base_ratio.max(1.0) * 1.02) as usize + TAPS + 64;
        // Lead-in of zeros gives the kernel's left wing something to read
        // before real audio arrives (and costs only HALF samples of delay).
        let mut buf = vec![0.0f32; cap];
        buf[..HALF].fill(0.0);
        Self {
            src_rate,
            dst_rate,
            base_ratio,
            kernel: Kernel::new(src_rate, dst_rate),
            ring_cap: ring_cap.max(1),
            target: (TARGET_MS / 1000.0 * src_rate as f64).max(64.0),
            buf,
            len: HALF,
            last_ratio: 1.0,
            last_need: 0,
            pos: HALF as f64,
            occ_avg: 0.0,
            catchup: 0.0,
            armed: false,
            ever: false,
            skip_pending: 0,
            gain: 1.0,
            dc_x1: 0.0,
            dc_y1: 0.0,
            dc_r: (1.0 - std::f64::consts::TAU * DC_HZ / dst_rate as f64) as f32,
        }
    }

    pub fn dst_rate(&self) -> u32 {
        self.dst_rate
    }

    /// True when the monitor runs at the chain rate (no rate conversion).
    pub fn native(&self) -> bool {
        self.src_rate == self.dst_rate
    }

    /// Render exactly `frames` output samples into `out` (silence-filled
    /// when the ring is short). Never allocates, never blocks.
    pub fn render(&mut self, src: &mut dyn Source, frames: usize, out: &mut [f32]) -> Report {
        let mut rep = Report::default();
        let frames = frames.min(out.len());
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(MAX_CHUNK);
            let r = self.chunk(src, n, &mut out[done..done + n]);
            rep.written += r.written;
            rep.underrun |= r.underrun;
            rep.dropped += r.dropped;
            rep.peak = rep.peak.max(r.peak);
            done += n;
        }
        rep
    }

    fn chunk(&mut self, src: &mut dyn Source, n: usize, out: &mut [f32]) -> Report {
        let mut rep = Report::default();
        let occ = src.occupied();

        // Pre-roll: let the tee build the target lead *plus one block* before
        // playing. The extra block is what the first pop borrows for the
        // kernel's right wing; without it the ring would park a block short
        // of target forever and the servo would run with a permanent droop.
        if !self.armed {
            let lead = self.target + n as f64 * self.base_ratio + TAPS as f64;
            let floor = if self.ever { lead * 0.5 } else { lead };
            if (occ as f64) < floor {
                out.fill(0.0);
                return rep;
            }
            self.armed = true;
            self.ever = true;
            // The first pop borrows one block + the kernel wing; whatever is
            // left after it is the trough this stream will hold. Adopt it as
            // the setpoint so the servo parks at zero error instead of
            // running with a permanent droop (a droop is a pitch offset).
            let est = n as f64 * self.base_ratio + HALF as f64 + 1.0;
            self.target = (occ as f64 - est).max(self.target * 0.75);
            self.occ_avg = self.target;
        }

        // Emergency trim: only a pathological producer can fill the ring
        // now (the catch-up servo below removes ~3%/s worth of excess), and
        // even then the skip is wrapped in a fade instead of a step.
        if self.skip_pending == 0
            && self.gain >= 1.0
            && occ > self.ring_cap * 9 / 10
        {
            self.skip_pending = occ.saturating_sub(self.target as usize / 2);
        }

        // Servo: the plant is an integrator (a ratio offset moves occupancy
        // at `src_rate` samples/s), so a plain proportional controller on the
        // *smoothed* occupancy is exactly right: it removes error with a 2 s
        // time constant, has no integral wind-up and therefore no slow pitch
        // hunting. Steady-state error for a real 0.1% clock drift is ~2 ms of
        // occupancy, and the pitch offset that corrects it is 0.1% (1.7
        // cents) — inaudible, and it is the drift itself.
        let err = (self.target - self.occ_avg) / (self.src_rate as f64 * 2.0);
        let servo = (1.0 - err).clamp(0.995, 1.005);
        // Walk *large* occupancy errors off over a second or so, capped at
        // 3% and slewed in/out so the pitch never jumps. A deadband keeps
        // the ordinary +-block quantization from modulating the ratio: the
        // servo above owns the steady state, this only rescues floods and
        // post-gap deficits.
        let dev = self.occ_avg / self.target - 1.0;
        let want = if dev > 0.5 {
            ((dev - 0.5) * 0.1).min(0.03)
        } else if dev < -0.5 {
            ((dev + 0.5) * 0.1).max(-0.015)
        } else {
            0.0
        };
        self.catchup += (want - self.catchup).clamp(-0.0004, 0.0004);
        let mut ratio = self.base_ratio * servo * (1.0 + self.catchup);
        if self.base_ratio == 1.0 && (ratio - 1.0).abs() < 1e-12 {
            ratio = 1.0;
        }

        // Pull the input this chunk will read (last output's right wing
        // included), leaving the rest in the ring for the next callback.
        let last = self.pos + (n.max(1) as f64 - 1.0) * ratio;
        let need = last.floor() as usize + HALF + 1;
        self.last_ratio = ratio;
        self.last_need = need;
        if need > self.buf.len() {
            // Cannot happen (MAX_CHUNK bounds `need`); silence rather than
            // allocating on the audio thread if it ever does.
            out.fill(0.0);
            return rep;
        }
        let want_in = need.saturating_sub(self.len);
        if want_in > 0 {
            let got = src.pop(&mut self.buf[self.len..self.len + want_in]);
            self.len += got;
        }
        // Setpoint and measurement must be the same point of the sawtooth:
        // measure the *trough* (just after the pop) and smooth it, so block
        // quantization never reaches the ratio. Feeding the peak instead
        // parks the servo half a block high and runs the monitor sharp.
        let occ_post = src.occupied() as f64;
        if self.occ_avg > 0.0 {
            self.occ_avg += 0.02 * (occ_post - self.occ_avg);
        } else {
            self.occ_avg = occ_post;
        }

        let mut i = 0;
        while i < n {
            let ip = self.pos.floor();
            if ip as usize + HALF >= self.len {
                break;
            }
            // Fade state machine (emergency trim only).
            if self.skip_pending > 0 {
                if self.gain > 0.0 {
                    self.gain = (self.gain - 1.0 / FADE as f32).max(0.0);
                } else {
                    let skipped = src.skip(self.skip_pending);
                    rep.dropped += skipped as u64;
                    self.skip_pending = 0;
                }
            } else if self.gain < 1.0 {
                self.gain = (self.gain + 1.0 / FADE as f32).min(1.0);
            }

            let v = self.kernel.at(&self.buf, self.pos);
            // DC blocker: y = x - x1 + R*y1 (5 Hz corner).
            let y = v - self.dc_x1 + self.dc_r * self.dc_y1;
            self.dc_x1 = v;
            self.dc_y1 = y;
            let s = y * self.gain;
            out[i] = s;
            let a = if s < 0.0 { -s } else { s };
            if a > rep.peak {
                rep.peak = a;
            }
            self.pos += ratio;
            i += 1;
        }
        rep.written = i;
        if i < n {
            out[i..n].fill(0.0);
            if self.armed {
                self.armed = false;
                rep.underrun = true;
            }
        }

        // Drop the consumed prefix, keeping HALF - 1 samples of history.
        let drop = (self.pos.floor() as usize).saturating_sub(HALF - 1);
        if drop > 0 {
            let keep = self.len - drop.min(self.len);
            self.buf.copy_within(drop..drop + keep, 0);
            self.len = keep;
            self.pos -= drop as f64;
        }
        rep
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resample::occupancy_servo;
    use ringbuf::traits::{Producer, Split};
    use crate::resample::resample_with_carry;

    /// A tee ring whose producer is driven explicitly by the test, so the
    /// virtual clock is exact and no thread scheduling enters the numbers.
    struct Feed {
        prod: ringbuf::HeapProd<f32>,
        cons: HeapCons<f32>,
    }

    impl Feed {
        fn new(cap: usize) -> Self {
            let (prod, cons) = ringbuf::HeapRb::<f32>::new(cap).split();
            Self { prod, cons }
        }
        fn push(&mut self, s: &[f32]) {
            let _ = self.prod.push_slice(s);
        }
    }

    /// The pre-fix monitor callback, reproduced exactly: pop
    /// `ceil(frames*ratio)+4`, linear-resample, zero-fill the shortfall.
    struct Legacy {
        ratio: f64,
        src_buf: Vec<f32>,
        mono: Vec<f32>,
        carry: (f64, f32, f32),
        servo_i: f64,
        primed: bool,
        target: usize,
        src_rate: u32,
        dst_rate: u32,
    }

    impl Legacy {
        fn new(src: u32, dst: u32) -> Self {
            let ratio = src as f64 / dst as f64;
            Self {
                ratio,
                src_buf: vec![0.0; (4096.0 * ratio) as usize + 64],
                mono: vec![0.0; 4096],
                carry: (0.0, 0.0, 0.0),
                servo_i: 0.0,
                primed: false,
                target: (TARGET_MS / 1000.0 * src as f64) as usize,
                src_rate: src,
                dst_rate: dst,
            }
        }

        fn render(&mut self, frames: usize, cons: &mut HeapCons<f32>) -> usize {
            let occ = cons.occupied_len();
            if occ < self.target {
                if !self.primed || occ < self.target / 2 {
                    self.primed = false;
                    self.mono[..frames].fill(0.0);
                    return 0;
                }
            }
            self.primed = true;
            if occ > self.target * 3 {
                let _ = Consumer::skip(cons, occ - self.target);
            }
            let occ = cons.occupied_len() as f64;
            let servo = occupancy_servo(&mut self.servo_i, occ, self.target as f64);
            let need = ((frames as f64 * self.ratio) * servo).ceil() as usize + 4;
            let got = cons.pop_slice(&mut self.src_buf[..need]);
            if got < need {
                self.src_buf[got..need].fill(0.0);
                self.primed = false;
            }
            let n = resample_with_carry(
                &mut self.carry,
                &self.src_buf[..need],
                self.src_rate,
                &mut self.mono[..frames],
                self.dst_rate,
                servo,
            );
            if n < frames {
                self.mono[n..frames].fill(0.0);
            }
            n
        }
    }

    fn sine(rate: f64, hz: f64, n: usize, phase: &mut f64) -> Vec<f32> {
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push((0.5 * (*phase).sin()) as f32);
            *phase += std::f64::consts::TAU * hz / rate;
        }
        v
    }

    /// Fundamental frequency from interpolated upward zero crossings.
    fn est_freq(x: &[f32], rate: f64) -> f64 {
        let mut crossings = 0usize;
        let mut first = None;
        let mut last = 0.0;
        for i in 1..x.len() {
            let (a, b) = (x[i - 1], x[i]);
            if a < 0.0 && b >= 0.0 {
                let t = i as f64 - 1.0 + (0.0 - a as f64) / (b as f64 - a as f64);
                if first.is_none() {
                    first = Some(t);
                } else {
                    crossings += 1;
                }
                last = t;
            }
        }
        match first {
            Some(f) if crossings > 0 && last > f => {
                crossings as f64 * rate / (last - f)
            }
            _ => 0.0,
        }
    }

    /// `(f0, amplitude, residual dB)` of the dominant tone: the residual is
    /// everything the monitor path added (clicks, sidebands, noise).
    fn analyze(x: &[f32], rate: f64) -> (f64, f64, f64) {
        let f0 = est_freq(x, rate);
        if f0 <= 0.0 {
            return (0.0, 0.0, f64::INFINITY);
        }
        // Refine by the phase difference between the two halves: for a pure
        // tone this is exact to well below the LS grid, which matters because
        // a 0.005 Hz error over 4 s alone shows up as a -35 dB "residual".
        let mut f = f0;
        let mid = x.len() / 2;
        let t = mid as f64 / rate;
        for _ in 0..3 {
            let p1 = fit_phase(&x[..mid], rate, f);
            let p2 = fit_phase(&x[mid..], rate, f);
            // Unwrap against the nominal frequency (thousands of cycles fit
            // in the baseline), then correct by the residual phase.
            let mut d = p2 - p1 - std::f64::consts::TAU * f * t;
            d = (d + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
                - std::f64::consts::PI;
            f += d / (std::f64::consts::TAU * t);
        }
        let (a, r) = fit(x, rate, f);
        let db = if a > 1e-12 {
            20.0 * (r / a).log10()
        } else {
            f64::INFINITY
        };
        (f, a, db)
    }

    /// Phase of a tone at `hz`, in the `cos(w n + phi)` convention, so a
    /// later block's phase advances by `+w * gap` and the two halves can be
    /// differenced without ambiguity.
    fn fit_phase(x: &[f32], rate: f64, hz: f64) -> f64 {
        let (a, b, _) = fit_parts(x, rate, hz);
        (-b).atan2(a)
    }

    fn fit(x: &[f32], rate: f64, hz: f64) -> (f64, f64) {
        let (a, b, res) = fit_parts(x, rate, hz);
        ((a * a + b * b).sqrt(), res)
    }

    /// Least-squares `(a, b, residual RMS)` for `a*cos + b*sin`.
    fn fit_parts(x: &[f32], rate: f64, hz: f64) -> (f64, f64, f64) {
        let w = std::f64::consts::TAU * hz / rate;
        let (mut scc, mut sss, mut scs) = (0.0, 0.0, 0.0);
        let (mut sxc, mut sxs) = (0.0, 0.0);
        for (i, &v) in x.iter().enumerate() {
            let (c, s) = ((w * i as f64).cos(), (w * i as f64).sin());
            let v = v as f64;
            scc += c * c;
            sss += s * s;
            scs += c * s;
            sxc += v * c;
            sxs += v * s;
        }
        let det = scc * sss - scs * scs;
        let (a, b) = if det.abs() > 1e-9 {
            (
                (sxc * sss - sxs * scs) / det,
                (sxs * scc - sxc * scs) / det,
            )
        } else {
            (0.0, 0.0)
        };
        let mut res = 0.0;
        for (i, &v) in x.iter().enumerate() {
            let fit = a * (w * i as f64).cos() + b * (w * i as f64).sin();
            let e = v as f64 - fit;
            res += e * e;
        }
        (a, b, (res / x.len() as f64).sqrt())
    }

    /// Goertzel amplitude of `hz` in `x` (leak-free for periodic tones).
    fn goertzel(x: &[f32], rate: f64, hz: f64) -> f64 {
        let n = x.len();
        let k = hz / rate * n as f64;
        let w = std::f64::consts::TAU * k / n as f64;
        let cw = w.cos();
        let coeff = 2.0 * cw;
        let (mut s0, mut s1, mut s2) = (0.0, 0.0, 0.0);
        for &v in x {
            s0 = v as f64 + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        let re = s1 - s2 * cw;
        let im = s2 * w.sin();
        // Normalise by N/2 for a real sinusoid of amplitude A.
        (re * re + im * im).sqrt() * 2.0 / n as f64
    }

    /// Drive a source at `src_block` frames per virtual 10 ms and a
    /// destination at `dst_block` frames per virtual 10 ms.
    fn run<F>(
        feed: &mut Feed,
        src_block: usize,
        dst_block: usize,
        steps: usize,
        hz: f64,
        mut render: F,
    ) -> Vec<f32>
    where
        F: FnMut(usize, &mut HeapCons<f32>, usize, &mut [f32]),
    {
        let mut phase = 0.0f64;
        let mut out = Vec::with_capacity(steps * dst_block);
        let mut block = vec![0.0f32; dst_block];
        for step in 0..steps {
            let s = sine(48_000.0, hz, src_block, &mut phase);
            feed.push(&s);
            render(step, &mut feed.cons, dst_block, &mut block);
            out.extend_from_slice(&block);
        }
        out
    }

    fn tail_db(x: &[f32], from: usize) -> &[f32] {
        &x[from.min(x.len())..]
    }

    #[test]
    fn ratio_path_sidebands() {
        // 1 kHz at 48 k -> 44.1 k: linear interpolation folds the 47/49 kHz
        // images onto 2.9/4.9 kHz — the audible "buzz" the user reported.
        let steps = 400; // 4 s of virtual time
        let src_block = 480; // 10 ms at 48 k
        let dst_block = 441; // 10 ms at 44.1 k

        let mut feed = Feed::new(24_000);
        let mut leg = Legacy::new(48_000, 44_100);
        let before = run(&mut feed, src_block, dst_block, steps, 1000.0, |_, cons, n, out| {
            let _ = leg.render(n, cons);
            out.copy_from_slice(&leg.mono[..n]);
        });

        let mut feed = Feed::new(24_000);
        let mut pipe = Pipe::new(48_000, 44_100, 24_000);
        let after = run(
            &mut feed,
            src_block,
            dst_block,
            steps,
            1000.0,
            |step, cons, n, out| {
                let r = pipe.render(cons, n, out);
                // Priming is silent by design; after that every block must
                // be fully synthesised (no zero-filled tails).
                if step > 20 {
                    assert_eq!(r.written, n, "block {step} short");
                }
            },
        );

        // Skip the 80 ms pre-roll so both measurements see steady state.
        // The residual is measured over the last 50 ms only: the drift servo
        // holds a ppm-level ratio offset, and over 4 s even 4 ppm smears a
        // single-tone fit (that smear is pitch, not distortion, and 4 ppm is
        // 0.007 cents). A short window isolates real artifacts.
        let skip = 20 * dst_block;
        let long_b = tail_db(&before, skip);
        let long_a = tail_db(&after, skip);
        let (_, b_amp, _) = analyze(long_b, 44_100.0);
        let (a_f, a_amp, a_db) = analyze(&long_a[long_a.len() - 2400..], 44_100.0);
        let a_29 = goertzel(long_a, 44_100.0, 2900.0);
        let a_49 = goertzel(long_a, 44_100.0, 4900.0);
        let db = |v: f64, r: f64| 20.0 * (v / r).log10();
        eprintln!(
            "48k->44.1k 1 kHz: legacy amp {b_amp:.4} (silent: phase rebase overruns the block) | \
             after f0 {a_f:.2} Hz amp {a_amp:.4} resid(50ms) {a_db:.1} dB, 2.9k {:.1} dB, 4.9k {:.1} dB",
            db(a_29, a_amp),
            db(a_49, a_amp),
        );
        assert!(a_db < -90.0, "residual {a_db:.1} dB too high");
        assert!(db(a_29, a_amp) < -90.0, "2.9 kHz image {:.1} dB", db(a_29, a_amp));
        assert!(db(a_49, a_amp) < -90.0, "4.9 kHz image {:.1} dB", db(a_49, a_amp));
        // The legacy path does not just colour the tone: its phase rebase
        // walks off the end of the block and it renders silence.
        assert!(b_amp < 1e-3, "legacy path should be dead, amp {b_amp}");
    }

    #[test]
    fn same_rate_has_no_click_train() {
        // The user's endpoint is 48 kHz and the chain is 48 kHz: no rate
        // conversion, yet the old callback dropped 4 samples per block.
        let steps = 400;
        let block = 480;
        let mut feed = Feed::new(24_000);
        let mut leg = Legacy::new(48_000, 48_000);
        let before = run(&mut feed, block, block, steps, 1000.0, |_, cons, n, out| {
            let _ = leg.render(n, cons);
            out.copy_from_slice(&leg.mono[..n]);
        });

        let mut feed = Feed::new(24_000);
        let mut pipe = Pipe::new(48_000, 48_000, 24_000);
        let after = run(&mut feed, block, block, steps, 1000.0, |step, cons, n, out| {
            let r = pipe.render(cons, n, out);
            if step > 20 {
                assert_eq!(r.written, n, "block {step} short");
            }
        });

        let skip = 20 * block;
        let (_, b_amp, b_db) = analyze(tail_db(&before, skip), 48_000.0);
        let short = &after[after.len() - 2400..];
        let (_, a_amp, a_db) = analyze(short, 48_000.0);
        let step = |x: &[f32]| {
            x.windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0f32, f32::max)
        };
        let ideal = (std::f64::consts::TAU * 1000.0 / 48_000.0 * 0.5) as f32;
        // A "click" is any step no sine at this level and frequency can make.
        let clicks = |x: &[f32]| {
            x.windows(2)
                .filter(|w| (w[1] - w[0]).abs() > ideal * 1.5)
                .count()
        };
        eprintln!(
            "48k->48k 1 kHz: before amp {b_amp:.4} resid {b_db:.1} dB max-step {:.4} clicks {} | \
             after amp {a_amp:.4} resid {a_db:.1} dB max-step {:.4} clicks {} (ideal {:.4})",
            step(tail_db(&before, skip)),
            clicks(tail_db(&before, skip)),
            step(tail_db(&after, skip)),
            clicks(tail_db(&after, skip)),
            ideal
        );
        assert!(a_db < -90.0);
        assert!(clicks(tail_db(&after, skip)) == 0, "steps in the new path");
        assert!(step(tail_db(&after, skip)) < ideal * 1.2);
        assert!(b_db > -40.0, "legacy click train should show, got {b_db:.1}");
        assert!(
            clicks(tail_db(&before, skip)) > 300,
            "legacy path should click every block"
        );
    }

    #[test]
    fn smooth_trim_never_steps() {
        // Flood the ring (2x producer) then let the servo walk it back.
        let mut feed = Feed::new(48_000);
        let mut pipe = Pipe::new(48_000, 48_000, 48_000);
        let mut phase = 0.0;
        let block = 480;
        let mut out = vec![0.0f32; block];
        let mut rendered = Vec::new();
        let mut worst_occ = 0usize;
        for step in 0..1600 {
            let s = sine(48_000.0, 1000.0, block, &mut phase);
            feed.push(&s);
            if step < 40 {
                feed.push(&s);
            }
            worst_occ = worst_occ.max(feed.cons.occupied_len());
            let r = pipe.render(&mut feed.cons, block, &mut out);
            if step > 20 {
                assert_eq!(r.written, block);
                rendered.extend_from_slice(&out);
            }
        }
        let (_, amp, db) = analyze(&rendered, 48_000.0);
        let max_step = rendered
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        let ideal = (std::f64::consts::TAU * 1000.0 / 48_000.0 * 0.5) as f32;
        eprintln!(
            "trim: peak occupancy {} samples, end {}, amp {amp:.3} resid {db:.1} dB, max-step {max_step:.4} (ideal {ideal:.4})",
            worst_occ,
            feed.cons.occupied_len()
        );
        assert!(feed.cons.occupied_len() as f64 <= pipe.target * 1.25);
        // A hard trim would show a step of up to a full sample's worth of
        // signal; the fade keeps every step inside the tone's own slope.
        assert!(max_step < ideal * 1.5, "step at a trim: {max_step}");
    }

    #[test]
    fn dc_is_blocked() {
        let mut feed = Feed::new(48_000);
        let mut pipe = Pipe::new(48_000, 48_000, 48_000);
        let mut out = vec![0.0f32; 480];
        let dc = vec![0.35f32; 480];
        let mut peak_tail = 0.0f32;
        for step in 0..200 {
            feed.push(&dc);
            pipe.render(&mut feed.cons, 480, &mut out);
            if step > 100 {
                peak_tail = peak_tail.max(out.iter().fold(0.0f32, |a, &v| a.max(v.abs())));
            }
        }
        eprintln!("dc: tail peak {peak_tail:.2e}");
        assert!(peak_tail < 1e-3, "DC leaked through: {peak_tail}");
    }

    #[test]
    fn native_flag_and_rates() {
        let p = Pipe::new(48_000, 48_000, 24_000);
        assert!(p.native());
        let p = Pipe::new(48_000, 44_100, 24_000);
        assert!(!p.native());
        assert_eq!(p.dst_rate(), 44_100);
    }
}
