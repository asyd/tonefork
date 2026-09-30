//! Test signals. Every mode is scaled to the same *RMS* level, so switching between a
//! sine, a band of noise and pink noise does not change the perceived loudness much.

use std::f32::consts::{SQRT_2, TAU};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

/// Octave band centers (ISO), a good default to walk through the spectrum.
pub const OCTAVES: [f32; 10] = [31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];

const FADE_IN_SECS: f32 = 1.0;
const FADE_OUT_SECS: f32 = 0.3;
const STEP_FADE_SECS: f32 = 0.06;

pub enum Kind {
    Sine(f32),
    Pink,
    /// Octave-wide band of pink noise around the frequency.
    Band(f32),
    /// Logarithmic sweep from `from` to `to` (Hz) over `secs` seconds, repeated.
    /// While playing, `Signal::position` holds the current frequency in Hz.
    Sweep { secs: f32, from: f32, to: f32 },
    /// Steps through the frequencies, `secs` each; sine or band noise.
    Steps { freqs: Vec<f32>, secs: f32, noise: bool },
}

struct Rng(u32);
impl Rng {
    fn next(&mut self) -> f32 {
        // xorshift32 -> [-1, 1)
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// Paul Kellet's economy pink-noise filter.
#[derive(Clone)]
struct Pink {
    b: [f32; 7],
    rng: Rng,
}
impl Clone for Rng {
    fn clone(&self) -> Self {
        Rng(self.0)
    }
}
impl Pink {
    fn new(seed: u32) -> Self {
        Pink { b: [0.0; 7], rng: Rng(seed.max(1)) }
    }
    fn next(&mut self) -> f32 {
        let w = self.rng.next();
        let b = &mut self.b;
        b[0] = 0.99886 * b[0] + w * 0.0555179;
        b[1] = 0.99332 * b[1] + w * 0.0750759;
        b[2] = 0.96900 * b[2] + w * 0.1538520;
        b[3] = 0.86650 * b[3] + w * 0.3104856;
        b[4] = 0.55000 * b[4] + w * 0.5329522;
        b[5] = -0.7616 * b[5] - w * 0.0168980;
        let out = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.5362;
        b[6] = w * 0.115926;
        out
    }
}

/// RBJ band-pass (constant 0 dB peak gain), Q = 1.414 -> about one octave wide.
#[derive(Clone)]
struct Bandpass {
    b0: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}
impl Bandpass {
    fn new(freq: f32, sr: f32) -> Self {
        let w0 = TAU * freq.min(sr * 0.45) / sr;
        let alpha = w0.sin() / (2.0 * 1.414);
        let a0 = 1.0 + alpha;
        Bandpass { b0: alpha / a0, b2: -alpha / a0, a1: -2.0 * w0.cos() / a0, a2: (1.0 - alpha) / a0, z1: 0.0, z2: 0.0 }
    }
    fn next(&mut self, x: f32) -> f32 {
        // transposed direct form II
        let y = self.b0 * x + self.z1;
        self.z1 = self.z2 - self.a1 * y;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Pink noise through a band-pass, normalized so its RMS is 1.
struct BandNoise {
    pink: Pink,
    filter: Bandpass,
    gain: f32,
}
impl BandNoise {
    fn new(freq: f32, sr: f32, seed: u32) -> Self {
        let (mut p, mut f) = (Pink::new(seed), Bandpass::new(freq, sr));
        let n = (sr * 2.0) as usize;
        let mut sum = 0.0f64;
        for i in 0..n + 4000 {
            let y = f.next(p.next());
            if i >= 4000 {
                sum += (y * y) as f64; // skip the filter warm-up
            }
        }
        let rms = (sum / n as f64).sqrt() as f32;
        BandNoise { pink: Pink::new(seed), filter: Bandpass::new(freq, sr), gain: 1.0 / rms.max(1e-6) }
    }
    fn next(&mut self) -> f32 {
        self.filter.next(self.pink.next()) * self.gain
    }
}

enum Engine {
    Sine { freq: f32, phase: f32 },
    Pink { pink: Pink, gain: f32 },
    Band(BandNoise),
    Sweep { phase: f32, pos: f32, secs: f32, from: f32, to: f32 },
    Steps { freqs: Vec<f32>, noises: Vec<BandNoise>, noise: bool, idx: usize, n: usize, seg: usize, phase: f32 },
}

pub struct Signal {
    sr: f32,
    /// Target RMS (linear, full scale = 1).
    level: f32,
    engine: Engine,
    fade_in: usize,
    fade_out: usize,
    played: usize,
    out_pos: Option<usize>,
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    /// Index of the current step (for `Steps`), read by the UI thread.
    pub position: Arc<AtomicUsize>,
}

impl Signal {
    pub fn new(kind: Kind, sr: f32, level_db: f32, stop: Arc<AtomicBool>, done: Arc<AtomicBool>) -> Self {
        let engine = match kind {
            Kind::Sine(freq) => Engine::Sine { freq, phase: 0.0 },
            Kind::Pink => {
                // Pink noise RMS is measured once so it matches the other modes.
                let mut p = Pink::new(1);
                let n = (sr * 2.0) as usize;
                let rms = ((0..n).map(|_| (p.next() as f64).powi(2)).sum::<f64>() / n as f64).sqrt() as f32;
                Engine::Pink { pink: Pink::new(7), gain: 1.0 / rms.max(1e-6) }
            }
            Kind::Band(f) => Engine::Band(BandNoise::new(f, sr, 11)),
            Kind::Sweep { secs, from, to } => Engine::Sweep { phase: 0.0, pos: 0.0, secs, from, to },
            Kind::Steps { freqs, secs, noise } => {
                let noises = if noise { freqs.iter().enumerate().map(|(i, &f)| BandNoise::new(f, sr, 100 + i as u32)).collect() } else { vec![] };
                Engine::Steps { freqs, noises, noise, idx: 0, n: 0, seg: (secs * sr) as usize, phase: 0.0 }
            }
        };
        Signal {
            sr,
            level: 10f32.powf(level_db / 20.0),
            engine,
            fade_in: (FADE_IN_SECS * sr) as usize,
            fade_out: (FADE_OUT_SECS * sr) as usize,
            played: 0,
            out_pos: None,
            stop,
            done,
            position: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn raw(&mut self) -> f32 {
        let (sr, level) = (self.sr, self.level);
        match &mut self.engine {
            Engine::Sine { freq, phase } => {
                *phase = (*phase + TAU * *freq / sr) % TAU;
                phase.sin() * SQRT_2 * level
            }
            Engine::Pink { pink, gain } => pink.next() * *gain * level,
            Engine::Band(b) => b.next() * level,
            Engine::Sweep { phase, pos, secs, from, to } => {
                let f = *from * (*to / *from).powf(*pos / *secs);
                self.position.store(f as usize, Ordering::Relaxed);
                *phase = (*phase + TAU * f / sr) % TAU;
                *pos += 1.0 / sr;
                if *pos >= *secs {
                    *pos = 0.0;
                }
                phase.sin() * SQRT_2 * level
            }
            Engine::Steps { freqs, noises, noise, idx, n, seg, phase } => {
                let fade = (STEP_FADE_SECS * sr) as usize;
                let env = if *n < fade {
                    *n as f32 / fade as f32
                } else if *n + fade > *seg {
                    (*seg - *n) as f32 / fade as f32
                } else {
                    1.0
                };
                let x = if *noise {
                    noises[*idx].next() * level
                } else {
                    *phase = (*phase + TAU * freqs[*idx] / sr) % TAU;
                    phase.sin() * SQRT_2 * level
                };
                *n += 1;
                if *n >= *seg {
                    *n = 0;
                    *idx = (*idx + 1) % freqs.len();
                    self.position.store(*idx, Ordering::Relaxed);
                }
                x * env
            }
        }
    }

    pub fn next(&mut self) -> f32 {
        if self.done.load(Ordering::Relaxed) {
            return 0.0;
        }
        // Master envelope: slow fade in, short fade out when asked to stop (no clicks).
        if self.out_pos.is_none() && self.stop.load(Ordering::Relaxed) {
            self.out_pos = Some(0);
        }
        let master = match self.out_pos {
            Some(p) if p >= self.fade_out => {
                self.done.store(true, Ordering::Relaxed);
                return 0.0;
            }
            Some(p) => {
                self.out_pos = Some(p + 1);
                1.0 - p as f32 / self.fade_out as f32
            }
            None => (self.played as f32 / self.fade_in as f32).min(1.0),
        };
        self.played = self.played.saturating_add(1);
        (self.raw() * master).clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn signal(kind: Kind, db: f32) -> (Signal, Arc<AtomicBool>, Arc<AtomicBool>) {
        let (stop, done) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        (Signal::new(kind, SR, db, stop.clone(), done.clone()), stop, done)
    }
    fn rms(s: &mut Signal, seconds: f32) -> f32 {
        let n = (SR * seconds) as usize;
        let sum: f64 = (0..n).map(|_| (s.next() as f64).powi(2)).sum();
        (sum / n as f64).sqrt() as f32
    }
    fn skip(s: &mut Signal, seconds: f32) {
        (0..(SR * seconds) as usize).for_each(|_| {
            s.next();
        });
    }

    #[test]
    fn sine_has_requested_frequency() {
        let (mut s, ..) = signal(Kind::Sine(1000.0), -20.0);
        skip(&mut s, 1.2); // past the fade-in
        let crossings = (0..SR as usize)
            .map({
                let mut prev = 0.0f32;
                move |_| {
                    let x = s.next();
                    let up = prev <= 0.0 && x > 0.0;
                    prev = x;
                    up
                }
            })
            .filter(|&u| u)
            .count();
        assert!((crossings as i32 - 1000).abs() <= 2, "{crossings}");
    }

    #[test]
    fn levels_match_across_modes() {
        for kind in [Kind::Sine(1000.0), Kind::Pink, Kind::Band(1000.0), Kind::Band(63.0), Kind::Band(8000.0)] {
            let (mut s, ..) = signal(kind, -20.0);
            skip(&mut s, 1.5);
            let db = 20.0 * rms(&mut s, 2.0).log10();
            assert!((db + 20.0).abs() < 1.5, "rms {db} dB");
        }
    }

    #[test]
    fn never_clips_and_fades_in() {
        let (mut s, ..) = signal(Kind::Sweep { secs: 2.0, from: 20.0, to: 20000.0 }, -12.0);
        let first = s.next().abs();
        assert!(first < 0.001, "starts at silence, got {first}");
        assert!((0..(SR * 5.0) as usize).all(|_| s.next().abs() <= 1.0));
    }

    #[test]
    fn stop_fades_out_then_signals_done() {
        let (mut s, stop, done) = signal(Kind::Sine(440.0), -20.0);
        skip(&mut s, 1.5);
        stop.store(true, Ordering::Relaxed);
        skip(&mut s, 0.5);
        assert!(done.load(Ordering::Relaxed));
        assert_eq!(s.next(), 0.0);
    }

    #[test]
    fn sweep_stays_in_its_range_and_reports_the_frequency() {
        let (mut s, ..) = signal(Kind::Sweep { secs: 4.0, from: 50.0, to: 150.0 }, -30.0);
        let mut seen = (usize::MAX, 0usize);
        for _ in 0..(SR * 4.0) as usize - 1 {
            s.next();
            let f = s.position.load(Ordering::Relaxed);
            seen = (seen.0.min(f), seen.1.max(f));
        }
        assert!(seen.0 >= 50 && seen.1 <= 150, "{seen:?}");
        assert!(seen.0 <= 51 && seen.1 >= 148, "covers the range: {seen:?}");
    }

    #[test]
    fn steps_advance_through_frequencies() {
        let (mut s, ..) = signal(Kind::Steps { freqs: vec![100.0, 1000.0, 5000.0], secs: 0.5, noise: false }, -30.0);
        assert_eq!(s.position.load(Ordering::Relaxed), 0);
        skip(&mut s, 0.6);
        assert_eq!(s.position.load(Ordering::Relaxed), 1);
        skip(&mut s, 1.0);
        assert_eq!(s.position.load(Ordering::Relaxed), 0); // wrapped after 3 steps
    }
}
