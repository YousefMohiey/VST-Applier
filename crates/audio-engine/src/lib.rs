pub mod convert;
pub mod engine;
pub mod lowlat;
mod lowlat_capture;
pub mod resample;
pub mod rt;

pub use engine::{
    input_devices, normalize_block_size, output_devices, start, ChainPlugin, Diagnostics,
    EngineError, Meters, RunningRoute, ALLOWED_BLOCKS,
};

/// Debug aid: low-latency render loop iteration count (spin bisection).
pub fn ll_iters() -> u64 {
    lowlat::LL_ITERS.load(std::sync::atomic::Ordering::Relaxed)
}
