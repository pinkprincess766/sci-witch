//! Microphone capture → 16 kHz mono WAV. The lead-in and the tail are cut
//! by [`crate::vad`], which measures against the recording's own noise
//! floor instead of a fixed amplitude.

use std::io::{self, BufRead};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;

use crate::error::{Error, Result};
use crate::vad;

pub const TARGET_HZ: u32 = 16_000;
const RECORDING_TEMP_PREFIX: &str = "sciwhisper-recording-";
const PREPARED_AUDIO_TEMP_PREFIX: &str = "sciwhisper-audio-";

/// Names of every input device the default host reports, best-effort (a
/// device whose name briefly fails to query is skipped, not fatal).
pub fn input_devices() -> Vec<String> {
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };
    devices.filter_map(|d| d.name().ok()).collect()
}

/// Resolves `name` to an input device. A device the user explicitly chose
/// is never silently swapped for another one: if it is absent or has been
/// unplugged, this fails instead of recording from whatever is left, so the
/// caller can surface that to the user rather than capture silently from
/// the wrong microphone.
fn select_device(name: Option<&str>) -> Result<cpal::Device> {
    let host = cpal::default_host();
    let Some(name) = name else {
        return host.default_input_device().ok_or(Error::NoMicrophone);
    };
    let devices = host
        .input_devices()
        .map_err(|e| Error::Audio(e.to_string()))?;
    devices
        .into_iter()
        .find(|d| d.name().is_ok_and(|n| n == name))
        .ok_or_else(|| {
            Error::Audio(format!(
                "выбранный микрофон «{name}» не найден или отключён; выберите другой в настройках или в трее"
            ))
        })
}

pub struct Recording {
    pub wav_path: PathBuf,
    pub duration_secs: f32,
    pub peak: f32,
    /// How far the speech stood above the room, in dB. Reported so a
    /// marginal recording can be named as such instead of silently
    /// producing a bad transcript.
    pub snr_db: f32,
    _temp_dir: tempfile::TempDir,
}

/// Audio prepared for Whisper. Converted files are removed when this value is dropped.
#[derive(Debug)]
pub struct PreparedAudio {
    path: PathBuf,
    _temp_dir: Option<tempfile::TempDir>,
}

impl PreparedAudio {
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// True when this value is responsible for deleting the file it points at.
    pub fn owns_temp_dir(&self) -> bool {
        self._temp_dir.is_some()
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
    pub fn start(device: Option<&str>) -> Result<Self> {
        let device = select_device(device)?;
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
    let resampled = vad::remove_dc(&resample(&mono, sample_rate, TARGET_HZ));
    let cfg = vad::VadConfig::default();
    let Some((trimmed, speech)) = vad::trim_to_speech(&resampled, TARGET_HZ, &cfg) else {
        return Err(Error::Audio(
            "тишина — ничего не произнесено (или микрофон выключен)".into(),
        ));
    };
    let peak = trimmed.iter().fold(0.0f32, |a, s| a.max(s.abs()));
    let wav = write_wav_with_prefix(&trimmed, TARGET_HZ, RECORDING_TEMP_PREFIX)?;
    Ok(Recording {
        wav_path: wav.path.clone(),
        duration_secs: trimmed.len() as f32 / TARGET_HZ as f32,
        peak,
        snr_db: speech.snr_db(),
        _temp_dir: wav
            ._temp_dir
            .expect("recorded audio always owns its temporary directory"),
    })
}

/// Record from `device` (or the system default, if `None`) until Enter, or
/// until `max_secs`.
pub fn record_wav(max_secs: Option<u64>, device: Option<&str>) -> Result<Recording> {
    let device = select_device(device)?;
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

/// Writes samples to a WAV inside a temporary directory the returned value
/// owns. Dropping the value removes the directory and the audio with it —
/// there is no separate cleanup step that an error path could skip.
pub fn write_temp_wav(samples: &[f32], hz: u32) -> Result<PreparedAudio> {
    write_wav(samples, hz)
}

fn write_wav(samples: &[f32], hz: u32) -> Result<PreparedAudio> {
    write_wav_with_prefix(samples, hz, PREPARED_AUDIO_TEMP_PREFIX)
}

fn write_wav_with_prefix(samples: &[f32], hz: u32, prefix: &str) -> Result<PreparedAudio> {
    let temp_dir = tempfile::Builder::new()
        .prefix(prefix)
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

/// Prepares an audio file for the recogniser: 16 kHz, mono, 16-bit WAV.
///
/// A WAV file is handled entirely in this crate — read, downmixed and
/// resampled here — so the common case needs no `ffmpeg` at all. That matters
/// for the portable Windows bundle, where asking a user to install a video
/// tool would defeat the point of shipping a self-contained pack.
///
/// Anything that is not a WAV still needs `ffmpeg` to decode it, and its
/// absence is reported plainly instead of handing the recogniser a file it
/// cannot read.
pub fn ensure_wav_16k(input: &std::path::Path) -> Result<PreparedAudio> {
    prepare_audio(input, which_ffmpeg())
}

/// The preparation step with `ffmpeg` injected, so its absence can be tested
/// without depending on what is installed on the machine running the tests.
pub fn prepare_audio_for_test(
    input: &std::path::Path,
    ffmpeg: Option<PathBuf>,
) -> Result<PreparedAudio> {
    prepare_audio(input, ffmpeg)
}

fn prepare_audio(input: &std::path::Path, ffmpeg: Option<PathBuf>) -> Result<PreparedAudio> {
    if !input.is_file() {
        return Err(Error::Audio(format!(
            "аудиофайл не найден: {}",
            input
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        )));
    }
    let is_wav = input
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"));

    // A file named .wav that this crate cannot read may still be something
    // ffmpeg understands, so a failure here falls through rather than refuses.
    if let (true, Ok((samples, hz))) = (is_wav, read_wav_samples(input)) {
        if hz == TARGET_HZ && already_mono_16bit(input) {
            // Nothing to change: hand the original file over and take no
            // ownership of it, so the caller's file is never deleted.
            return Ok(PreparedAudio {
                path: input.to_path_buf(),
                _temp_dir: None,
            });
        }
        return write_wav(&resample(&samples, hz, TARGET_HZ), TARGET_HZ);
    }

    let Some(ffmpeg) = ffmpeg else {
        return Err(Error::Audio(format!(
            "файл {} не является 16 kHz WAV, а для его преобразования нужен ffmpeg, \
             которого нет в системе. Запишите звук через саму программу — \
             для записи с микрофона ffmpeg не нужен — или преобразуйте файл заранее.",
            input
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        )));
    };

    let temp_dir = tempfile::Builder::new()
        .prefix("sciwhisper-")
        .tempdir()
        .map_err(|e| Error::Message(e.to_string()))?;
    let out = temp_dir.path().join("audio.wav");
    let status = std::process::Command::new(ffmpeg)
        .args(["-y", "-i"])
        .arg(input)
        .args(["-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le"])
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if !status.success() {
        return Err(Error::Audio(
            "ffmpeg не смог преобразовать аудиофайл".into(),
        ));
    }
    Ok(PreparedAudio {
        path: out,
        _temp_dir: Some(temp_dir),
    })
}

/// Reads a WAV into mono samples, whatever its channel count and bit depth.
/// Mono samples and their rate, as stored — no resampling. Used by
/// [`crate::corpus`], which must measure the file it was given rather than a
/// converted copy of it.
pub fn read_wav_samples_for_corpus(path: &std::path::Path) -> Result<(Vec<f32>, u32)> {
    read_wav_samples(path)
}

fn read_wav_samples(path: &std::path::Path) -> Result<(Vec<f32>, u32)> {
    let mut reader = hound::WavReader::open(path).map_err(|e| Error::Audio(e.to_string()))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let scale = match spec.bits_per_sample {
        8 => i8::MAX as f32,
        16 => i16::MAX as f32,
        24 => 8_388_607.0,
        _ => i32::MAX as f32,
    };
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Audio(e.to_string()))?,
        hound::SampleFormat::Int => reader
            .samples::<i32>()
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Audio(e.to_string()))?
            .into_iter()
            .map(|value| value as f32 / scale)
            .collect(),
    };
    let mono = if channels <= 1 {
        interleaved
    } else {
        interleaved
            .chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
            .collect()
    };
    Ok((mono, spec.sample_rate))
}

fn already_mono_16bit(path: &std::path::Path) -> bool {
    hound::WavReader::open(path)
        .map(|reader| {
            let spec = reader.spec();
            spec.channels == 1
                && spec.bits_per_sample == 16
                && spec.sample_format == hound::SampleFormat::Int
        })
        .unwrap_or(false)
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

    #[test]
    fn missing_configured_microphone_fails_instead_of_silently_substituting() {
        // A device the user explicitly chose must never be silently swapped
        // for another one (e.g. after it is unplugged): the caller has to
        // see this as an error, not start recording from the wrong mic.
        let err = select_device(Some("это устройство точно не существует #12345"));
        assert!(err.is_err());
    }

    #[test]
    fn no_configured_microphone_falls_back_to_system_default_without_error() {
        // Absence of a preference is not the same as a missing preference:
        // `None` should still resolve (or fail only if there is truly no
        // input device at all), never because of the name-matching branch.
        let result = select_device(None);
        if let Err(e) = result {
            assert!(matches!(e, Error::NoMicrophone));
        }
    }
}
