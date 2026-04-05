//! Safe wrapper around `nnnoiseless::DenoiseState`.
//!
//! Model weights and C sources are bundled inside the `nnnoiseless` crate —
//! no submodule, no external files, no build scripts needed.

use nnnoiseless::DenoiseState;

/// nnnoiseless processes audio in 480-sample frames (10ms @ 48kHz, mono).
pub const FRAME_SIZE: usize = nnnoiseless::FRAME_SIZE;

pub struct Denoiser {
    state: Box<DenoiseState<'static>>,
    out_buf: Vec<f32>,
}

// Safe to send across threads when guarded by Mutex
unsafe impl Send for Denoiser {}

impl Denoiser {
    pub fn new() -> Self {
        Self {
            state: DenoiseState::new(),
            out_buf: vec![0.0f32; FRAME_SIZE],
        }
    }

    /// Denoise one FRAME_SIZE-sample frame in-place.
    /// Returns the Voice Activity Detection (VAD) probability in [0.0, 1.0].
    pub fn process_frame(&mut self, pcm: &mut [f32; FRAME_SIZE]) -> f32 {
        let vad = self.state.process_frame(&mut self.out_buf, pcm);
        pcm.copy_from_slice(&self.out_buf);
        vad
    }
}

impl Default for Denoiser {
    fn default() -> Self {
        Self::new()
    }
}
