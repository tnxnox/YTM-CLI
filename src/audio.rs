use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use anyhow::Result;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};

// ---------------------------------------------------------------------------
// Shared Visualizer State
// ---------------------------------------------------------------------------

pub struct VisualizerShared {
    // 8 frequency bands storing f32 values via bits as AtomicU32
    pub bands: [AtomicU32; 8],
}

impl VisualizerShared {
    pub fn new() -> Self {
        Self {
            bands: [
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
            ],
        }
    }

    pub fn get_band(&self, idx: usize) -> f32 {
        if idx < 8 {
            f32::from_bits(self.bands[idx].load(Ordering::Relaxed))
        } else {
            0.0
        }
    }

    pub fn set_band(&self, idx: usize, val: f32) {
        if idx < 8 {
            self.bands[idx].store(val.to_bits(), Ordering::Relaxed);
        }
    }
}

// ---------------------------------------------------------------------------
// Sample Conversion Trait
// ---------------------------------------------------------------------------

pub trait ToF32 {
    fn to_f32(self) -> f32;
}

impl ToF32 for i16 {
    #[inline]
    fn to_f32(self) -> f32 {
        self as f32 / 32768.0
    }
}

impl ToF32 for u16 {
    #[inline]
    fn to_f32(self) -> f32 {
        (self as f32 - 32768.0) / 32768.0
    }
}

impl ToF32 for f32 {
    #[inline]
    fn to_f32(self) -> f32 {
        self
    }
}

// ---------------------------------------------------------------------------
// Visualizer Source (DSP crossover network + envelope followers)
// ---------------------------------------------------------------------------

pub struct VisualizerSource<S> {
    inner: S,
    shared: Arc<VisualizerShared>,
    lp_states: [f32; 7],
    envs: [f32; 8],
}

impl<S> VisualizerSource<S> {
    pub fn new(inner: S, shared: Arc<VisualizerShared>) -> Self {
        Self {
            inner,
            shared,
            lp_states: [0.0; 7],
            envs: [0.0; 8],
        }
    }
}

impl<S> Iterator for VisualizerSource<S>
where
    S: Source,
    S::Item: rodio::Sample + ToF32 + Copy,
{
    type Item = S::Item;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let sample_opt = self.inner.next();
        if let Some(sample) = sample_opt {
            let x = sample.to_f32();

            // First-order Low-pass Filter cascade (calculated for fs=44100Hz)
            // Cutoffs: ~80Hz, ~200Hz, ~500Hz, ~1000Hz, ~2500Hz, ~6000Hz, ~12000Hz
            let lp0 = self.lp_states[0] + 0.011 * (x - self.lp_states[0]);
            let lp1 = self.lp_states[1] + 0.027 * (x - self.lp_states[1]);
            let lp2 = self.lp_states[2] + 0.066 * (x - self.lp_states[2]);
            let lp3 = self.lp_states[3] + 0.125 * (x - self.lp_states[3]);
            let lp4 = self.lp_states[4] + 0.263 * (x - self.lp_states[4]);
            let lp5 = self.lp_states[5] + 0.461 * (x - self.lp_states[5]);
            let lp6 = self.lp_states[6] + 0.631 * (x - self.lp_states[6]);

            self.lp_states[0] = lp0;
            self.lp_states[1] = lp1;
            self.lp_states[2] = lp2;
            self.lp_states[3] = lp3;
            self.lp_states[4] = lp4;
            self.lp_states[5] = lp5;
            self.lp_states[6] = lp6;

            // Subtractive crossover frequency bands
            let b0 = lp0;
            let b1 = lp1 - lp0;
            let b2 = lp2 - lp1;
            let b3 = lp3 - lp2;
            let b4 = lp4 - lp3;
            let b5 = lp5 - lp4;
            let b6 = lp6 - lp5;
            let b7 = x - lp6;

            let bands = [b0, b1, b2, b3, b4, b5, b6, b7];

            // Apply envelope followers & update shared memory
            for i in 0..8 {
                let abs_b = bands[i].abs();
                let env = &mut self.envs[i];
                let alpha = if abs_b > *env { 0.15 } else { 0.003 }; // fast attack, slow decay
                *env = *env + alpha * (abs_b - *env);
                self.shared.set_band(i, *env);
            }

            Some(sample)
        } else {
            None
        }
    }
}

impl<S> Source for VisualizerSource<S>
where
    S: Source,
    S::Item: rodio::Sample + ToF32 + Copy,
{
    #[inline]
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }

    #[inline]
    fn channels(&self) -> u16 {
        self.inner.channels()
    }

    #[inline]
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }

    #[inline]
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    #[inline]
    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        self.inner.try_seek(pos)
    }
}

// ---------------------------------------------------------------------------
// Audio Player
// ---------------------------------------------------------------------------

pub struct AudioPlayer {
    _stream: OutputStream,
    pub stream_handle: OutputStreamHandle,
}

impl AudioPlayer {
    pub fn new() -> Result<Self> {
        let (_stream, stream_handle) = OutputStream::try_default()?;
        Ok(Self {
            _stream,
            stream_handle,
        })
    }

    pub fn play_local(&self, file_path: PathBuf) -> Result<(Sink, Duration, Arc<VisualizerShared>)> {
        let file = File::open(file_path)?;
        let source = Decoder::new(file)?;
        let total_duration = source.total_duration().unwrap_or(Duration::from_secs(0));
        let shared = Arc::new(VisualizerShared::new());
        let vis_source = VisualizerSource::new(source, Arc::clone(&shared));
        let sink = Sink::try_new(&self.stream_handle)?;
        sink.append(vis_source);
        Ok((sink, total_duration, shared))
    }
}
