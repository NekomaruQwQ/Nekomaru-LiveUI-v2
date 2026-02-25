//! Capture session wrapper and viewport calculation.
//!
//! Re-exports `winrt_capture::CaptureSession` and provides the
//! aspect-ratio-preserving viewport calculation used by the main loop.

pub use winrt_capture::CaptureSession;

use nkcore::prelude::euclid::Size2D;
use windows::Win32::Graphics::Direct3D11::D3D11_VIEWPORT;

/// Compute a viewport that fits `source_size` into `target_size` with
/// aspect-ratio-preserving letterboxing.
///
/// The result is a `D3D11_VIEWPORT` centered within `target_size`, scaled
/// uniformly so the source fills as much of the target as possible without
/// stretching.
pub fn calculate_resample_viewport(
    source_size: Size2D<u32>,
    target_size: Size2D<u32>) -> D3D11_VIEWPORT {
    let scale =
        f32::min(
            target_size.width as f32 / source_size.width as f32,
            target_size.height as f32 / source_size.height as f32);
    let source_size_scaled =
        (source_size.to_f32() * scale).floor().to_u32();
    let target_offset =
        (target_size - source_size_scaled).to_vector() / 2;

    D3D11_VIEWPORT {
        TopLeftX: target_offset.x as _,
        TopLeftY: target_offset.y as _,
        Width: source_size_scaled.width as _,
        Height: source_size_scaled.height as _,
        MinDepth: 0.0,
        MaxDepth: 1.0,
    }
}
