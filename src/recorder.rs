use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};

pub struct Recorder {
    state: State,
}

enum State {
    Idle,
    Recording {
        _stream: cpal::Stream,
        samples: Arc<Mutex<Vec<f32>>>,
        sample_rate: u32,
        channels: u16,
    },
}

// cpal::Stream holds a CoreAudio raw pointer but is safe to send between threads:
// all audio I/O happens on CoreAudio's internal threads; the Stream handle just
// keeps the unit alive and can be dropped from any thread safely.
unsafe impl Send for Recorder {}
unsafe impl Sync for Recorder {}

impl Default for Recorder {
    fn default() -> Self {
        Self { state: State::Idle }
    }
}

impl Recorder {
    pub fn start(&mut self) -> Result<()> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("no audio input device found")?;

        let config = device
            .default_input_config()
            .context("failed to get default input config")?;

        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
        let samples_cb = samples.clone();

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => build_stream::<f32>(&device, &config.into(), samples_cb)?,
            cpal::SampleFormat::I16 => build_stream::<i16>(&device, &config.into(), samples_cb)?,
            cpal::SampleFormat::U16 => build_stream::<u16>(&device, &config.into(), samples_cb)?,
            _ => anyhow::bail!("unsupported sample format"),
        };

        stream.play().context("failed to start audio stream")?;
        tracing::info!(sample_rate, channels, "recording started");

        self.state = State::Recording {
            _stream: stream,
            samples,
            sample_rate,
            channels,
        };
        Ok(())
    }

    /// Stops recording and returns 16 kHz mono f32 samples.
    pub fn stop(&mut self) -> Vec<f32> {
        let State::Recording {
            samples,
            sample_rate,
            channels,
            ..
        } = std::mem::replace(&mut self.state, State::Idle)
        else {
            return vec![];
        };
        // Dropping _stream stops the capture.

        let raw = std::mem::take(&mut *samples.lock().unwrap());
        tracing::info!(samples = raw.len(), "recording stopped");

        to_16k_mono(raw, channels, sample_rate)
    }

    pub fn is_recording(&self) -> bool {
        matches!(self.state, State::Recording { .. })
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    samples: Arc<Mutex<Vec<f32>>>,
) -> Result<cpal::Stream>
where
    T: cpal::Sample + cpal::SizedSample + Into<f32> + Send + 'static,
{
    let channels = config.channels as usize;
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _| {
            let mut buf = samples.lock().unwrap();
            for chunk in data.chunks(channels) {
                let mono: f32 =
                    chunk.iter().map(|s| -> f32 { (*s).into() }).sum::<f32>() / channels as f32;
                buf.push(mono);
            }
        },
        |e| tracing::error!(error = %e, "audio input error"),
        None,
    )?;
    Ok(stream)
}

fn to_16k_mono(raw: Vec<f32>, channels: u16, from_rate: u32) -> Vec<f32> {
    // Already mono and correct rate — no-op
    if channels == 1 && from_rate == 16_000 {
        return raw;
    }

    // Mix to mono if needed
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
