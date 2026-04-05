//! 3-band EQ (Bass / Mid / Treble) applied after RNNoise.
//!
//! Uses biquad filters (Direct Form II Transposed).
//! Band gains are controlled via atomics so the UI can tweak them in real-time.

use std::f64::consts::PI;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// Master EQ on/off
pub static EQ_ENABLED: AtomicBool = AtomicBool::new(true);

// ─── Atomic EQ band gains (stored as dB × 10, e.g. +30 = +3.0 dB) ─────────
/// Bass  — low-shelf  @ 300 Hz  (default +3.0 dB)
pub static EQ_BASS_DB10: AtomicI32 = AtomicI32::new(30);
/// Mid   — peaking    @ 2.5 kHz (default +1.5 dB)
pub static EQ_MID_DB10: AtomicI32 = AtomicI32::new(15);
/// Treble — high-shelf @ 6 kHz  (default -2.5 dB)
pub static EQ_TREBLE_DB10: AtomicI32 = AtomicI32::new(-25);

pub fn load_eq_db(atom: &AtomicI32) -> f64 {
    atom.load(Ordering::Relaxed) as f64 / 10.0
}

// ─── Biquad filter ──────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Biquad {
    b0: f64, b1: f64, b2: f64,
    a1: f64, a2: f64,
    z1: f64, z2: f64,
}

impl Biquad {
    pub fn low_shelf(freq: f64, gain_db: f64, sample_rate: f64) -> Self {
        let a = 10f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / 2.0 * ((a + 1.0 / a) * (1.0 / 0.9 - 1.0) + 2.0).sqrt();

        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * alpha * a.sqrt();
        Self {
            b0: a * ((a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * alpha * a.sqrt()) / a0,
            b1: 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0) / a0,
            b2: a * ((a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * alpha * a.sqrt()) / a0,
            a1: -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0) / a0,
            a2: ((a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * alpha * a.sqrt()) / a0,
            z1: 0.0, z2: 0.0,
        }
    }

    pub fn high_shelf(freq: f64, gain_db: f64, sample_rate: f64) -> Self {
        let a = 10f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / 2.0 * ((a + 1.0 / a) * (1.0 / 0.9 - 1.0) + 2.0).sqrt();

        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * alpha * a.sqrt();
        Self {
            b0: a * ((a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * alpha * a.sqrt()) / a0,
            b1: -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0) / a0,
            b2: a * ((a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * alpha * a.sqrt()) / a0,
            a1: 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0) / a0,
            a2: ((a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * alpha * a.sqrt()) / a0,
            z1: 0.0, z2: 0.0,
        }
    }

    pub fn peaking(freq: f64, gain_db: f64, q: f64, sample_rate: f64) -> Self {
        let a = 10f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);

        let a0 = 1.0 + alpha / a;
        Self {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cos_w0) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha / a) / a0,
            z1: 0.0, z2: 0.0,
        }
    }

    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

// ─── 3-Band EQ ──────────────────────────────────────────────────────────────

pub struct WarmthEQ {
    sample_rate: f64,
    bass: Biquad,
    mid: Biquad,
    treble: Biquad,
    // Track current dB to know when to rebuild filters
    cur_bass_db10: i32,
    cur_mid_db10: i32,
    cur_treble_db10: i32,
}

impl WarmthEQ {
    pub fn new(sample_rate: f64) -> Self {
        let bass_db = load_eq_db(&EQ_BASS_DB10);
        let mid_db = load_eq_db(&EQ_MID_DB10);
        let treble_db = load_eq_db(&EQ_TREBLE_DB10);

        Self {
            sample_rate,
            bass: Biquad::low_shelf(300.0, bass_db, sample_rate),
            mid: Biquad::peaking(2500.0, mid_db, 1.2, sample_rate),
            treble: Biquad::high_shelf(6000.0, treble_db, sample_rate),
            cur_bass_db10: EQ_BASS_DB10.load(Ordering::Relaxed),
            cur_mid_db10: EQ_MID_DB10.load(Ordering::Relaxed),
            cur_treble_db10: EQ_TREBLE_DB10.load(Ordering::Relaxed),
        }
    }

    /// Process a frame. Automatically picks up atomic dB changes from UI.
    #[inline]
    pub fn process_frame(&mut self, frame: &mut [f32]) {
        // Check if any band changed — rebuild only the changed filter(s)
        let new_bass = EQ_BASS_DB10.load(Ordering::Relaxed);
        let new_mid = EQ_MID_DB10.load(Ordering::Relaxed);
        let new_treble = EQ_TREBLE_DB10.load(Ordering::Relaxed);

        if new_bass != self.cur_bass_db10 {
            self.cur_bass_db10 = new_bass;
            self.bass = Biquad::low_shelf(300.0, new_bass as f64 / 10.0, self.sample_rate);
        }
        if new_mid != self.cur_mid_db10 {
            self.cur_mid_db10 = new_mid;
            self.mid = Biquad::peaking(2500.0, new_mid as f64 / 10.0, 1.2, self.sample_rate);
        }
        if new_treble != self.cur_treble_db10 {
            self.cur_treble_db10 = new_treble;
            self.treble = Biquad::high_shelf(6000.0, new_treble as f64 / 10.0, self.sample_rate);
        }

        // All bands at 0 dB? Skip processing entirely.
        if new_bass == 0 && new_mid == 0 && new_treble == 0 {
            return;
        }

        for s in frame.iter_mut() {
            let mut x = *s as f64;
            if self.cur_bass_db10 != 0 { x = self.bass.process(x); }
            if self.cur_mid_db10 != 0 { x = self.mid.process(x); }
            if self.cur_treble_db10 != 0 { x = self.treble.process(x); }
            *s = x as f32;
        }
    }

    pub fn reset(&mut self) {
        self.bass.reset();
        self.mid.reset();
        self.treble.reset();
    }
}
