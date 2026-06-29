use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};

// cpal::Stream holds a CoreAudio raw pointer but is safe to send between threads:
// all audio I/O happens on CoreAudio's internal threads; the Stream handle just
// keeps the unit alive and can be dropped from any thread safely.
unsafe impl Send for Recorder {}
unsafe impl Sync for Recorder {}

/// Shared between the stream callback and the Recorder.
/// `None`  → callback is running but discarding samples (standby).
/// `Some`  → accumulate into the Vec (recording).
type Buf = Arc<Mutex<Option<Vec<f32>>>>;

struct Live {
    _stream: cpal::Stream,
    buf: Buf,
    sample_rate: u32,
    channels: u16,
}

#[derive(Default)]
pub struct Recorder {
    live: Option<Live>,
}

impl Recorder {
    /// Open the CoreAudio stream now so the first `start()` is instant.
    /// Safe to call multiple times; no-ops if already warm.
    pub fn warm(&mut self) -> Result<()> {
        if self.live.is_some() {
            return Ok(());
        }
        self.live = Some(open_stream()?);
        Ok(())
    }

    /// Begin accumulating samples. Opens the stream if not already warmed.
    pub fn start(&mut self) -> Result<()> {
        if self.live.is_none() {
            self.live = Some(open_stream()?);
        }
        *self.live.as_ref().unwrap().buf.lock().unwrap() = Some(Vec::new());
        Ok(())
    }

    /// Stop accumulating and return 16 kHz mono samples.
    pub fn stop(&mut self) -> Vec<f32> {
        let Some(live) = &self.live else {
            return vec![];
        };
        let raw = live
            .buf
            .lock()
            .unwrap()
            .take() // puts None back → standby mode
            .unwrap_or_default();
        tracing::info!(samples = raw.len(), "recording stopped");
        to_16k_mono(raw, live.channels, live.sample_rate)
    }

    pub fn is_recording(&self) -> bool {
        self.live
            .as_ref()
            .is_some_and(|l| l.buf.lock().unwrap().is_some())
    }
}

fn open_stream() -> Result<Live> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .context("no audio input device found")?;

    let config = device
        .default_input_config()
        .context("failed to get default input config")?;

    let sample_rate = config.sample_rate().0;
    let channels = config.channels();
    let buf: Buf = Arc::new(Mutex::new(None));
    let buf_cb = buf.clone();

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build_stream::<f32>(&device, &config.into(), buf_cb, channels)?,
        cpal::SampleFormat::I16 => build_stream::<i16>(&device, &config.into(), buf_cb, channels)?,
        cpal::SampleFormat::U16 => build_stream::<u16>(&device, &config.into(), buf_cb, channels)?,
        _ => anyhow::bail!("unsupported sample format"),
    };

    stream.play().context("failed to start audio stream")?;
    tracing::info!(sample_rate, channels, "audio stream warmed");

    Ok(Live {
        _stream: stream,
        buf,
        sample_rate,
        channels,
    })
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buf: Buf,
    channels: u16,
) -> Result<cpal::Stream>
where
    T: cpal::Sample + cpal::SizedSample + Into<f32> + Send + 'static,
{
    let ch = channels as usize;
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _| {
            let mut guard = buf.lock().unwrap();
            if let Some(acc) = guard.as_mut() {
                for chunk in data.chunks(ch) {
                    let mono: f32 =
                        chunk.iter().map(|s| -> f32 { (*s).into() }).sum::<f32>() / ch as f32;
                    acc.push(mono);
                }
            }
            // None → discard (standby)
        },
        |e| tracing::error!(error = %e, "audio input error"),
        None,
    )?;
    Ok(stream)
}

fn to_16k_mono(raw: Vec<f32>, channels: u16, from_rate: u32) -> Vec<f32> {
    if channels == 1 && from_rate == 16_000 {
        return raw;
    }

    let mono: Vec<f32> = if channels == 1 {
        raw
    } else {
        raw.chunks_exact(channels as usize)
            .map(|c| c.iter().sum::<f32>() / channels as f32)
            .collect()
    };

    if from_rate == 16_000 {
        return mono;
    }

    resample(mono, from_rate, 16_000)
}

fn resample(input: Vec<f32>, from: u32, to: u32) -> Vec<f32> {
    use rubato::{
        Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
    };

    let params = SincInterpolationParameters {
        sinc_len: 128,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Nearest,
        oversampling_factor: 128,
        window: WindowFunction::Hann,
    };

    let ratio = to as f64 / from as f64;
    let len = input.len();

    let mut resampler =
        SincFixedIn::<f32>::new(ratio, 2.0, params, len, 1).expect("resampler init failed");

    let mut out = resampler.process(&[input], None).expect("resample failed");
    out.remove(0)
}
