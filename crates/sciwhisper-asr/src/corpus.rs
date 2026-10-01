//! Facts about a recording, for the research corpus.
//!
//! A voice corpus is only reproducible if every record says which bytes it
//! was measured on. These are the fields `dataset_schema_version: 2` asks
//! for, read from the file itself rather than typed by whoever collected
//! it: the checksum, the format, the duration, and how far the speech stood
//! above the room.
//!
//! Nothing here transcribes. Recognition is a separate step with a separate
//! failure mode, and mixing the two would make an unreadable file look like
//! a recogniser problem.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::{capture, model, vad};

/// What a corpus record needs to say about its audio.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioFacts {
    pub sha256: String,
    pub duration_secs: f64,
    pub sample_rate_hz: u32,
    pub channels: u32,
    /// `None` when the detector finds no speech at all — the recording is
    /// then reported as unusable rather than given a made-up number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snr_db: Option<f64>,
    pub has_speech: bool,
}

/// Reads `path` and measures it. WAV only: a research corpus that stores
/// lossy audio cannot tell a codec artefact from a recognition error.
pub fn describe_wav(path: &Path) -> Result<AudioFacts> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if extension != "wav" {
        return Err(Error::Audio(format!(
            "{} is not a WAV file; the corpus stores uncompressed audio so that a codec artefact cannot be mistaken for a recognition error",
            path.display()
        )));
    }
    let reader = hound::WavReader::open(path).map_err(|e| Error::Audio(e.to_string()))?;
    let spec = reader.spec();
    let frames = reader.duration() as f64;
    drop(reader);
    if spec.sample_rate == 0 {
        return Err(Error::Audio(format!(
            "{} declares a sample rate of zero",
            path.display()
        )));
    }

    let sha256 = model::sha256_file(path).map_err(|e| Error::Audio(e.to_string()))?;
    let (samples, hz) = capture::read_wav_samples_for_corpus(path)?;
    let speech = vad::detect(&vad::remove_dc(&samples), hz, &vad::VadConfig::default());

    Ok(AudioFacts {
        sha256,
        duration_secs: frames / f64::from(spec.sample_rate),
        sample_rate_hz: spec.sample_rate,
        channels: u32::from(spec.channels.max(1)),
        snr_db: speech
            .has_speech()
            .then(|| f64::from(speech.snr_db()))
            .filter(|value| value.is_finite()),
        has_speech: speech.has_speech(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, samples: &[f32], hz: u32) -> std::path::PathBuf {
        let path = dir.join(name);
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: hz,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for sample in samples {
            writer
                .write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .unwrap();
        }
        writer.finalize().unwrap();
        path
    }

    fn spoken(hz: u32) -> Vec<f32> {
        let mut state: u32 = 5;
        (0..hz as usize * 2)
            .map(|i| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let room = ((state >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0) * 0.0004;
                let t = i as f32 / hz as f32;
                if i > hz as usize / 2 && i < hz as usize * 3 / 2 {
                    room + (t * 2.0 * std::f32::consts::PI * 150.0).sin() * 0.05
                } else {
                    room
                }
            })
            .collect()
    }

    #[test]
    fn a_recording_is_described_from_its_own_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "a.wav", &spoken(16_000), 16_000);
        let facts = describe_wav(&path).unwrap();
        assert_eq!(facts.sample_rate_hz, 16_000);
        assert_eq!(facts.channels, 1);
        assert!((facts.duration_secs - 2.0).abs() < 0.01);
        assert_eq!(facts.sha256.len(), 64);
        assert!(facts.has_speech);
        assert!(facts.snr_db.unwrap() > 20.0);
    }

    #[test]
    fn the_same_bytes_describe_identically() {
        let dir = tempfile::tempdir().unwrap();
        let samples = spoken(16_000);
        let a = describe_wav(&write(dir.path(), "a.wav", &samples, 16_000)).unwrap();
        let b = describe_wav(&write(dir.path(), "b.wav", &samples, 16_000)).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn a_recording_of_a_room_is_reported_as_holding_no_speech() {
        let dir = tempfile::tempdir().unwrap();
        let mut state: u32 = 11;
        let room: Vec<f32> = (0..32_000)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0) * 0.05
            })
            .collect();
        let facts = describe_wav(&write(dir.path(), "room.wav", &room, 16_000)).unwrap();
        assert!(!facts.has_speech);
        assert_eq!(facts.snr_db, None);
    }

    #[test]
    fn lossy_audio_is_refused_rather_than_guessed_at() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mp3");
        std::fs::write(&path, b"not really an mp3").unwrap();
        let error = describe_wav(&path).unwrap_err().to_string();
        assert!(error.contains("WAV"), "{error}");
    }
}
