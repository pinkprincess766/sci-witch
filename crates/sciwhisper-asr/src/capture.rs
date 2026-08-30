//! Microphone capture → 16 kHz mono WAV. VAD-style silence trim is applied
//! after resampling: leading/trailing near-zero samples are dropped.

use std::io::{self, BufRead};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;

use crate::error::{Error, Result};

pub const TARGET_HZ: u32 = 16_000;

pub struct Recording {
    pub wav_path: PathBuf,
    pub duration_secs: f32,
    pub peak: f32,
    _temp_dir: tempfile::TempDir,
}

/// Audio prepared for Whisper. Converted files are removed when this value is dropped.
pub struct PreparedAudio {
    path: PathBuf,
    _temp_dir: Option<tempfile::TempDir>,
}

impl PreparedAudio {
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

/// Push-to-talk session: start on key-down, finish on key-up.
pub struct PttSession {
    stop: Arc<AtomicBool>,
    buf: Arc<Mutex<Vec<f32>>>,
    stream: cpal::Stream,
    sample_rate: u32,
    channels: usize,
}

impl PttSession {
    pub fn start() -> Result<Self> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or(Error::NoMicrophone)?;
        let config = device
            .default_input_config()
            .map_err(|e| Error::Audio(e.to_string()))?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels() as usize;
        let buf: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let stream = match config.sample_format() {
            SampleFormat::F32 => {
                build_stream::<f32>(&device, &config.into(), &buf, &stop, channels)?
            }
            SampleFormat::I16 => {
                build_stream::<i16>(&device, &config.into(), &buf, &stop, channels)?
            }
            SampleFormat::U16 => {
                build_stream::<u16>(&device, &config.into(), &buf, &stop, channels)?
            }
            other => {
                return Err(Error::Audio(format!("unsupported sample format {other:?}")));
            }
        };
        stream.play().map_err(|e| Error::Audio(e.to_string()))?;
        Ok(Self {
            stop,
            buf,
            stream,
            sample_rate,
            channels,
        })
    }

    pub fn finish(self) -> Result<Recording> {
        self.stop.store(true, Ordering::SeqCst);
        drop(self.stream);
        finalize_samples(self.buf, self.sample_rate, self.channels)
    }

    pub fn cancel(self) {
        self.stop.store(true, Ordering::SeqCst);
        drop(self.stream);
    }
}

fn finalize_samples(
    buf: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
    channels: usize,
) -> Result<Recording> {
    let samples = buf.lock().unwrap().clone();
    if samples.len() < (sample_rate as usize / 10) {
        return Err(Error::Audio("recording too short".into()));
    }
    let mono = if channels <= 1 {
        samples
    } else {
        samples
            .chunks(channels)
            .map(|c| c.iter().sum::<f32>() / c.len() as f32)
            .collect()
    };
    let resampled = resample(&mono, sample_rate, TARGET_HZ);
    let trimmed = trim_silence(&resampled, 0.01);
    let peak = trimmed.iter().fold(0.0f32, |a, s| a.max(s.abs()));
    if peak < 0.005 {
        return Err(Error::Audio(
            "тишина — ничего не произнесено (или микрофон выключен)".into(),
        ));
    }
    let wav = write_wav(&trimmed, TARGET_HZ)?;
    Ok(Recording {
        wav_path: wav.path.clone(),
        duration_secs: trimmed.len() as f32 / TARGET_HZ as f32,
        peak,
        _temp_dir: wav
            ._temp_dir
            .expect("recorded audio always owns its temporary directory"),
    })
}

/// Record from the default microphone until Enter, or until `max_secs`.
pub fn record_wav(max_secs: Option<u64>) -> Result<Recording> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or(Error::NoMicrophone)?;
    let config = device
        .default_input_config()
        .map_err(|e| Error::Audio(e.to_string()))?;
    let sample_rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    let buf: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));

    let stream = match config.sample_format() {
        SampleFormat::F32 => build_stream::<f32>(&device, &config.into(), &buf, &stop, channels)?,
        SampleFormat::I16 => build_stream::<i16>(&device, &config.into(), &buf, &stop, channels)?,
        SampleFormat::U16 => build_stream::<u16>(&device, &config.into(), &buf, &stop, channels)?,
        other => {
            return Err(Error::Audio(format!("unsupported sample format {other:?}")));
        }
    };
    stream.play().map_err(|e| Error::Audio(e.to_string()))?;

    if let Some(secs) = max_secs {
        eprintln!("запись {secs} с");
    } else {
        eprintln!("говорите — Enter остановит запись");
    }

    if let Some(secs) = max_secs {
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(secs) {
            thread::sleep(Duration::from_millis(30));
        }
        stop.store(true, Ordering::SeqCst);
    } else {
        let mut line = String::new();
        let _ = io::stdin().lock().read_line(&mut line);
        stop.store(true, Ordering::SeqCst);
    }
    drop(stream);
    finalize_samples(buf, sample_rate, channels)
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buf: &Arc<Mutex<Vec<f32>>>,
    stop: &Arc<AtomicBool>,
    channels: usize,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample + ToF32,
{
    let buf = buf.clone();
    let stop = stop.clone();
    let err_fn = |e| eprintln!("mic error: {e}");
    let _ = channels;
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let mut g = buf.lock().unwrap();
                g.extend(data.iter().map(|s| s.to_f32()));
            },
            err_fn,
            None,
        )
        .map_err(|e| Error::Audio(e.to_string()))
}

trait ToF32 {
    fn to_f32(self) -> f32;
}

impl ToF32 for f32 {
    fn to_f32(self) -> f32 {
        self
    }
}
impl ToF32 for i16 {
    fn to_f32(self) -> f32 {
        self as f32 / i16::MAX as f32
    }
}
impl ToF32 for u16 {
    fn to_f32(self) -> f32 {
        (self as f32 / u16::MAX as f32) * 2.0 - 1.0
    }
}

fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let n = ((input.len() as f64) / ratio).floor() as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let src = i as f64 * ratio;
        let j = src.floor() as usize;
        let frac = (src - j as f64) as f32;
        let a = input[j];
        let b = *input.get(j + 1).unwrap_or(&a);
        out.push(a + (b - a) * frac);
    }
    out
}

fn trim_silence(samples: &[f32], thresh: f32) -> Vec<f32> {
    let Some(start) = samples.iter().position(|s| s.abs() > thresh) else {
        return vec![];
    };
    let end = samples
        .iter()
        .rposition(|s| s.abs() > thresh)
        .unwrap_or(start);
    samples[start..=end].to_vec()
}

fn write_wav(samples: &[f32], hz: u32) -> Result<PreparedAudio> {
    let temp_dir = tempfile::Builder::new()
        .prefix("sciwhisper-")
        .tempdir()
        .map_err(|e| Error::Message(e.to_string()))?;
    let path = temp_dir.path().join("audio.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: hz,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&path, spec).map_err(|e| Error::Audio(e.to_string()))?;
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        w.write_sample(v).map_err(|e| Error::Audio(e.to_string()))?;
    }
    w.finalize().map_err(|e| Error::Audio(e.to_string()))?;
    Ok(PreparedAudio {
        path,
        _temp_dir: Some(temp_dir),
    })
}

/// Resample an existing audio file to 16 kHz mono WAV via ffmpeg if needed.
pub fn ensure_wav_16k(input: &std::path::Path) -> Result<PreparedAudio> {
    let ext = input
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "wav" {
        // still resample — whisper is happiest at 16 kHz
    }
    if which_ffmpeg().is_none() {
        return Ok(PreparedAudio {
            path: input.to_path_buf(),
            _temp_dir: None,
        });
    }
    let temp_dir = tempfile::Builder::new()
        .prefix("sciwhisper-")
        .tempdir()
        .map_err(|e| Error::Message(e.to_string()))?;
    let out = temp_dir.path().join("audio.wav");
    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-i"])
        .arg(input)
        .args(["-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le"])
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if !status.success() {
        return Err(Error::Audio("ffmpeg failed to convert audio".into()));
    }
    Ok(PreparedAudio {
        path: out,
        _temp_dir: Some(temp_dir),
    })
}

fn which_ffmpeg() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join("ffmpeg"))
            .find(|c| c.is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporary_recording_is_removed_on_drop() {
        let wav = write_wav(&[0.1; 1_600], TARGET_HZ).unwrap();
        let path = wav.path().to_path_buf();
        assert!(path.exists());
        drop(wav);
        assert!(!path.exists());
    }
}
