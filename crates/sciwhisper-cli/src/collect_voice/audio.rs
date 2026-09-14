//! Collector capture retains silence and bounds memory independently of stdin.
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use super::store::{err, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use sciwhisper_asr::capture::{self, PreparedAudio};

pub fn microphones() -> Result<Vec<String>> {
    cpal::default_host()
        .input_devices()
        .map_err(err)?
        .map(|d| d.name().map_err(err))
        .collect()
}

#[derive(Default)]
struct Buffer {
    mono: Vec<f32>,
    error: Option<String>,
}

fn append<T: Sample + Copy>(buffer: &mut Buffer, data: &[T], channels: usize, limit: usize)
where
    f32: FromSample<T>,
{
    for frame in data
        .chunks_exact(channels)
        .take(limit.saturating_sub(buffer.mono.len()))
    {
        let value = frame.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / channels as f32;
        if !value.is_finite() {
            buffer.error = Some("Микрофон вернул некорректные отсчёты".into());
            return;
        }
        buffer.mono.push(value);
    }
}

fn stream<T: SizedSample + Copy>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buffer: Arc<Mutex<Buffer>>,
    frames: usize,
) -> Result<cpal::Stream>
where
    f32: FromSample<T>,
{
    let errors = buffer.clone();
    let channels = usize::from(config.channels);
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                if let Ok(mut buffer) = buffer.lock() {
                    append(&mut buffer, data, channels, frames);
                }
            },
            move |error| {
                if let Ok(mut buffer) = errors.lock() {
                    buffer.error = Some(format!("Запись прервана: {error}"));
                }
            },
            None,
        )
        .map_err(err)
}

pub fn record(
    mic: &str,
    seconds: u64,
    input: &mpsc::Receiver<String>,
) -> Result<Option<PreparedAudio>> {
    if !(1..=60).contains(&seconds) {
        return Err("Длительность записи: 1–60 секунд".into());
    }
    let device = cpal::default_host()
        .input_devices()
        .map_err(err)?
        .find(|d| d.name().is_ok_and(|n| n == mic))
        .ok_or("Выбранный микрофон отключён; запись не началась")?;
    let supported = device.default_input_config().map_err(err)?;
    let config: cpal::StreamConfig = supported.clone().into();
    if config.channels == 0
        || config.channels > 32
        || !(8000..=192000).contains(&config.sample_rate.0)
    {
        return Err("Неподдерживаемый формат микрофона".into());
    }
    let frames = config.sample_rate.0 as usize * seconds as usize;
    let buffer = Arc::new(Mutex::new(Buffer::default()));
    let stream = match supported.sample_format() {
        SampleFormat::F32 => stream::<f32>(&device, &config, buffer.clone(), frames)?,
        SampleFormat::I16 => stream::<i16>(&device, &config, buffer.clone(), frames)?,
        SampleFormat::U16 => stream::<u16>(&device, &config, buffer.clone(), frames)?,
        other => return Err(format!("Неподдерживаемый формат: {other:?}")),
    };
    stream.play().map_err(err)?;
    println!("Запись {seconds} с. Enter — остановить, q + Enter — отменить.");
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        match input.recv_timeout(Duration::from_millis(30)) {
            Ok(line) if line.trim() == "q" => return Ok(None),
            Ok(_) => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(None),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if let Some(error) = &buffer.lock().map_err(err)?.error {
            return Err(error.clone());
        }
    }
    drop(stream);
    let buffer = buffer.lock().map_err(err)?;
    if let Some(error) = &buffer.error {
        return Err(error.clone());
    }
    if buffer.mono.len() < config.sample_rate.0 as usize / 10 {
        return Err("Запись слишком короткая".into());
    }
    // Both values own temporary files. Return an independent 16 kHz copy,
    // including when conversion borrows the original file unchanged.
    let original = capture::write_temp_wav(&buffer.mono, config.sample_rate.0).map_err(err)?;
    let converted = capture::ensure_wav_16k(original.path()).map_err(err)?;
    let (samples, hz) = capture::read_wav_samples_for_corpus(converted.path()).map_err(err)?;
    capture::write_temp_wav(&samples, hz).map(Some).map_err(err)
}

pub fn playback(path: &Path) -> Result<()> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = Command::new("powershell.exe");
        c.args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; $p = New-Object System.Media.SoundPlayer; $p.SoundLocation=$env:SCI_WITCH_PLAYBACK; $p.Load(); $p.PlaySync()"]);
        c.env("SCI_WITCH_PLAYBACK", path);
        c
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = Command::new("/usr/bin/afplay");
        c.arg(path);
        c
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut command = {
        let mut c = Command::new("aplay");
        c.arg(path);
        c
    };
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Не удалось открыть проигрыватель: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(65);
    loop {
        if let Some(status) = child.try_wait().map_err(err)? {
            return if status.success() {
                Ok(())
            } else {
                Err("Проигрыватель не смог воспроизвести запись".into())
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Проигрыватель превысил время ожидания".into());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_is_bounded_downmixed_and_preserves_silence() {
        let mut b = Buffer::default();
        append(&mut b, &[0f32, 0., 1., -1., 0.5, 0.5, 1., 1.], 2, 3);
        append(&mut b, &[1f32; 100], 2, 3);
        assert_eq!(b.mono, vec![0., 0., 0.5]);
    }
    #[test]
    fn invalid_samples_are_an_error_not_a_successful_recording() {
        let mut b = Buffer::default();
        append(&mut b, &[f32::NAN], 1, 10);
        assert!(b.error.is_some());
    }
}
