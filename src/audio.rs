use anyhow::Result;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

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
    sample_count: usize,
}

impl<S> VisualizerSource<S> {
    pub fn new(inner: S, shared: Arc<VisualizerShared>) -> Self {
        Self {
            inner,
            shared,
            lp_states: [0.0; 7],
            envs: [0.0; 8],
            sample_count: 0,
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
            }

            self.sample_count += 1;
            if self.sample_count >= 512 {
                for i in 0..8 {
                    self.shared.set_band(i, self.envs[i]);
                }
                self.sample_count = 0;
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
    fn try_seek(&mut self, pos: Duration) -> std::result::Result<(), rodio::source::SeekError> {
        self.inner.try_seek(pos)
    }
}

// ---------------------------------------------------------------------------
// Custom Seekable Media Source Wrapper
// ---------------------------------------------------------------------------

struct MySeekableSource {
    file: File,
    len: u64,
}

impl Read for MySeekableSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}

impl Seek for MySeekableSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(pos)
    }
}

impl symphonia::core::io::MediaSource for MySeekableSource {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

// ---------------------------------------------------------------------------
// Custom Symphonia Decoder supporting seeking via proper byte_len
// ---------------------------------------------------------------------------

pub struct SymphoniaDecoder {
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    current_frame_offset: usize,
    format: Box<dyn symphonia::core::formats::FormatReader>,
    total_duration: Option<symphonia::core::units::Time>,
    buffer: symphonia::core::audio::SampleBuffer<i16>,
    buffer_duration: u64,
    spec: symphonia::core::audio::SignalSpec,
}

fn get_buffer(
    decoded: symphonia::core::audio::AudioBufferRef,
    spec: &symphonia::core::audio::SignalSpec,
) -> symphonia::core::audio::SampleBuffer<i16> {
    let duration = symphonia::core::units::Duration::from(decoded.capacity() as u64);
    let mut buffer = symphonia::core::audio::SampleBuffer::<i16>::new(duration, *spec);
    buffer.copy_interleaved_ref(decoded);
    buffer
}

fn skip_back_a_tiny_bit(
    symphonia::core::units::Time {
        mut seconds,
        mut frac,
    }: symphonia::core::units::Time,
) -> symphonia::core::units::Time {
    frac -= 0.0001;
    if frac < 0.0 {
        seconds = seconds.saturating_sub(1);
        frac = 1.0 - frac;
    }
    symphonia::core::units::Time { seconds, frac }
}

impl SymphoniaDecoder {
    pub fn new(file: File, extension: Option<&str>) -> Result<Self> {
        let len = file.metadata()?.len();
        let source = MySeekableSource { file, len };
        let mss = symphonia::core::io::MediaSourceStream::new(Box::new(source), Default::default());

        let mut hint = symphonia::core::probe::Hint::new();
        if let Some(ext) = extension {
            hint.with_extension(ext);
        }

        let format_opts = symphonia::core::formats::FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let metadata_opts = symphonia::core::meta::MetadataOptions::default();
        let mut probed =
            symphonia::default::get_probe().format(&hint, mss, &format_opts, &metadata_opts)?;

        let track = probed
            .format
            .default_track()
            .or_else(|| {
                probed
                    .format
                    .tracks()
                    .iter()
                    .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
            })
            .ok_or_else(|| anyhow::anyhow!("No supported audio track found"))?
            .clone();

        let track_id = track.id;

        let mut decoder = symphonia::default::get_codecs().make(
            &track.codec_params,
            &symphonia::core::codecs::DecoderOptions::default(),
        )?;

        let total_duration = track
            .codec_params
            .time_base
            .zip(track.codec_params.n_frames)
            .map(|(base, frames)| base.calc_time(frames));

        // Decode first packet to initialize buffer and spec
        let packet = loop {
            let p = probed.format.next_packet()?;
            if p.track_id() == track_id {
                break p;
            }
        };

        let decoded = decoder.decode(&packet)?;
        let spec = decoded.spec().to_owned();
        let buffer_duration = decoded.capacity() as u64;
        let buffer = get_buffer(decoded, &spec);

        Ok(Self {
            decoder,
            current_frame_offset: 0,
            format: probed.format,
            total_duration,
            buffer,
            buffer_duration,
            spec,
        })
    }

    fn refine_position(
        &mut self,
        seek_res: symphonia::core::formats::SeekedTo,
    ) -> std::result::Result<(), symphonia::core::errors::Error> {
        let mut samples_to_pass = seek_res.required_ts - seek_res.actual_ts;
        let packet = loop {
            let candidate = self.format.next_packet()?;
            if candidate.dur() > samples_to_pass {
                break candidate;
            } else {
                samples_to_pass -= candidate.dur();
            }
        };

        let mut decoded = self.decoder.decode(&packet);
        for _ in 0..3 {
            if decoded.is_err() {
                let packet = self.format.next_packet()?;
                decoded = self.decoder.decode(&packet);
            }
        }

        let decoded = decoded?;
        let spec = decoded.spec().to_owned();
        let capacity = decoded.capacity() as u64;
        if self.buffer_duration < capacity
            || self.spec.rate != spec.rate
            || self.spec.channels != spec.channels
        {
            let duration = symphonia::core::units::Duration::from(capacity);
            self.buffer = symphonia::core::audio::SampleBuffer::<i16>::new(duration, spec);
            self.buffer_duration = capacity;
        }
        self.spec = spec;
        self.buffer.copy_interleaved_ref(decoded);
        self.current_frame_offset = samples_to_pass as usize * self.spec.channels.count() as usize;
        Ok(())
    }
}

impl Iterator for SymphoniaDecoder {
    type Item = i16;

    #[inline]
    fn next(&mut self) -> Option<i16> {
        if self.current_frame_offset >= self.buffer.len() {
            let packet = self.format.next_packet().ok()?;
            let mut decoded = self.decoder.decode(&packet);
            for _ in 0..3 {
                if decoded.is_err() {
                    let packet = self.format.next_packet().ok()?;
                    decoded = self.decoder.decode(&packet);
                }
            }
            let decoded = decoded.ok()?;
            let spec = decoded.spec().to_owned();
            let capacity = decoded.capacity() as u64;
            if self.buffer_duration < capacity
                || self.spec.rate != spec.rate
                || self.spec.channels != spec.channels
            {
                let duration = symphonia::core::units::Duration::from(capacity);
                self.buffer = symphonia::core::audio::SampleBuffer::<i16>::new(duration, spec);
                self.buffer_duration = capacity;
            }
            self.spec = spec;
            self.buffer.copy_interleaved_ref(decoded);
            self.current_frame_offset = 0;
        }

        let sample = *self.buffer.samples().get(self.current_frame_offset)?;
        self.current_frame_offset += 1;

        Some(sample)
    }
}

impl Source for SymphoniaDecoder {
    #[inline]
    fn current_frame_len(&self) -> Option<usize> {
        Some(self.buffer.len().saturating_sub(self.current_frame_offset))
    }

    #[inline]
    fn channels(&self) -> u16 {
        self.spec.channels.count() as u16
    }

    #[inline]
    fn sample_rate(&self) -> u32 {
        self.spec.rate
    }

    #[inline]
    fn total_duration(&self) -> Option<Duration> {
        self.total_duration
            .map(|symphonia::core::units::Time { seconds, frac }| {
                Duration::new(seconds, (frac * 1_000_000_000.0) as u32)
            })
    }

    fn try_seek(&mut self, pos: Duration) -> std::result::Result<(), rodio::source::SeekError> {
        use symphonia::core::formats::{SeekMode, SeekTo};

        let seek_beyond_end = self
            .total_duration
            .map(|symphonia::core::units::Time { seconds, frac }| {
                Duration::new(seconds, (frac * 1_000_000_000.0) as u32)
            })
            .is_some_and(|dur| dur.saturating_sub(pos).as_millis() < 1);

        let time = if seek_beyond_end {
            let time = self.total_duration.expect("if guarantees this is Some");
            skip_back_a_tiny_bit(time) // some decoders can only seek to just before the end
        } else {
            pos.as_secs_f64().into()
        };

        // make sure the next sample is for the right channel
        let to_skip = self.current_frame_offset % self.spec.channels.count() as usize;

        let seek_res = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: None,
                },
            )
            .map_err(|e| rodio::source::SeekError::Other(Box::new(e)))?;

        self.refine_position(seek_res)
            .map_err(|e| rodio::source::SeekError::Other(Box::new(e)))?;

        self.current_frame_offset += to_skip;

        Ok(())
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

    pub fn play_local(
        &self,
        file_path: PathBuf,
    ) -> Result<(Sink, Duration, Arc<VisualizerShared>)> {
        let file = File::open(&file_path)?;
        let ext = file_path.extension().and_then(|e| e.to_str());
        let source = SymphoniaDecoder::new(file, ext)?;
        let total_duration = source.total_duration().unwrap_or(Duration::from_secs(0));
        let shared = Arc::new(VisualizerShared::new());
        let vis_source = VisualizerSource::new(source, Arc::clone(&shared));
        let sink = Sink::try_new(&self.stream_handle)?;
        sink.append(vis_source);
        Ok((sink, total_duration, shared))
    }
}
