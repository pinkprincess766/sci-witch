//! Voice activity detection over 16 kHz mono samples.
//!
//! The recorder used to keep everything between the first and last sample
//! louder than a fixed 0.01, and to declare silence when the peak stayed
//! under 0.005. Both numbers are absolute, so the same recording passed or
//! failed depending on the microphone's gain: a quiet headset was reported
//! as silence, and a room with a fan was never trimmed at all.
//!
//! This module decides in decibels relative to the recording's own noise
//! floor instead. It is a pure function over samples — no device, no
//! files — so every rule below is covered by a test with a synthetic
//! signal.
//!
//! What it deliberately does not do: remove pauses *inside* speech. A
//! person dictating a formula stops to think, and splicing those gaps out
//! would run words together for the recogniser. Internal audio is kept as
//! recorded; only the lead-in and the tail are cut.

use std::ops::Range;

/// Frame length and hop in milliseconds, plus every threshold the decision
/// depends on. Public so a caller can loosen the rules for a noisy room
/// without editing this file.
#[derive(Clone, Copy, Debug)]
pub struct VadConfig {
    pub frame_ms: usize,
    pub hop_ms: usize,
    /// Which quantile of frame energies is taken to be the noise floor.
    pub noise_quantile: f32,
    /// How far above the floor a frame must sit to open a segment.
    pub enter_margin_db: f32,
    /// How far above the floor a frame may sink before the segment closes.
    /// Lower than `enter_margin_db` on purpose: opening is harder than
    /// staying open, so one quiet syllable does not cut a word in half.
    pub exit_margin_db: f32,
    /// A frame this quiet is never speech, whatever the floor says. Guards
    /// against a recording that is pure numeric noise, where the floor
    /// itself is meaninglessly low.
    pub absolute_floor_db: f32,
    /// If the whole recording is this flat, it is stationary noise and
    /// carries no speech, however loud it is.
    pub min_dynamic_range_db: f32,
    /// A burst shorter than this is a click, a chair, a key press.
    pub min_speech_ms: usize,
    /// How long a segment stays open after the last speech frame.
    pub hangover_ms: usize,
    /// How much audio is kept before the first speech frame.
    pub lead_pad_ms: usize,
    /// Segments closer than this are one utterance with a pause in it.
    pub merge_gap_ms: usize,
    /// Total speech below this means the recording holds no utterance.
    pub min_total_speech_ms: usize,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            frame_ms: 20,
            hop_ms: 10,
            noise_quantile: 0.10,
            enter_margin_db: 9.0,
            exit_margin_db: 5.0,
            absolute_floor_db: -70.0,
            min_dynamic_range_db: 8.0,
            min_speech_ms: 120,
            hangover_ms: 240,
            lead_pad_ms: 160,
            merge_gap_ms: 320,
            min_total_speech_ms: 150,
        }
    }
}

/// One stretch of speech, as sample indices into the input, padding included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Speech {
    pub start: usize,
    pub end: usize,
}

impl Speech {
    pub fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
}

/// What the detector saw. The measurements are kept alongside the verdict
/// so an error message can say *why* a recording was rejected.
#[derive(Clone, Debug)]
pub struct VadResult {
    pub segments: Vec<Speech>,
    pub speech_samples: usize,
    pub noise_floor_db: f32,
    pub loud_db: f32,
}

impl VadResult {
    pub fn has_speech(&self) -> bool {
        !self.segments.is_empty()
    }

    /// The extent to keep: from the start of the first segment to the end
    /// of the last. Pauses between segments stay in the audio.
    pub fn span(&self) -> Option<Range<usize>> {
        let first = self.segments.first()?;
        let last = self.segments.last()?;
        Some(first.start..last.end)
    }

    /// Signal-to-noise of the loud part over the floor, in dB. A recording
    /// that only just clears the thresholds is worth warning about.
    pub fn snr_db(&self) -> f32 {
        self.loud_db - self.noise_floor_db
    }
}

fn db(rms: f32) -> f32 {
    20.0 * (rms.max(1e-12)).log10()
}

fn frame_rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum: f32 = frame.iter().map(|s| s * s).sum();
    (sum / frame.len() as f32).sqrt()
}

/// Fraction of adjacent sample pairs that change sign. High for fricatives
/// («с», «ш», «ф»), low for vowels and for low-frequency rumble.
fn zero_crossing_rate(frame: &[f32]) -> f32 {
    if frame.len() < 2 {
        return 0.0;
    }
    let crossings = frame
        .windows(2)
        .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
        .count();
    crossings as f32 / (frame.len() - 1) as f32
}

/// Subtracts the mean. Some capture chains carry a DC offset large enough
/// to dominate the RMS of a quiet frame, which would raise the estimated
/// noise floor above the speech itself.
pub fn remove_dc(samples: &[f32]) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    let mean = samples.iter().map(|s| *s as f64).sum::<f64>() / samples.len() as f64;
    let mean = mean as f32;
    samples.iter().map(|s| s - mean).collect()
}

fn quantile(sorted: &[f32], q: f32) -> f32 {
    if sorted.is_empty() {
        return f32::NEG_INFINITY;
    }
    let idx = ((sorted.len() - 1) as f32 * q.clamp(0.0, 1.0)).round() as usize;
    sorted[idx]
}

struct Frames {
    db: Vec<f32>,
    zcr: Vec<f32>,
    frame: usize,
    hop: usize,
}

fn analyse(samples: &[f32], hz: u32, cfg: &VadConfig) -> Frames {
    let frame = (hz as usize * cfg.frame_ms / 1000).max(1);
    let hop = (hz as usize * cfg.hop_ms / 1000).max(1);
    let mut energies = Vec::new();
    let mut zcrs = Vec::new();
    let mut start = 0;
    while start < samples.len() {
        let end = (start + frame).min(samples.len());
        let window = &samples[start..end];
        energies.push(db(frame_rms(window)));
        zcrs.push(zero_crossing_rate(window));
        if end == samples.len() {
            break;
        }
        start += hop;
    }
    Frames {
        db: energies,
        zcr: zcrs,
        frame,
        hop,
    }
}

/// Marks the speech in `samples`, which must already be mono at `hz`.
pub fn detect(samples: &[f32], hz: u32, cfg: &VadConfig) -> VadResult {
    let frames = analyse(samples, hz, cfg);
    if frames.db.is_empty() {
        return VadResult {
            segments: Vec::new(),
            speech_samples: 0,
            noise_floor_db: f32::NEG_INFINITY,
            loud_db: f32::NEG_INFINITY,
        };
    }

    let mut sorted = frames.db.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let floor = quantile(&sorted, cfg.noise_quantile);
    let loud = quantile(&sorted, 0.95);

    let empty = VadResult {
        segments: Vec::new(),
        speech_samples: 0,
        noise_floor_db: floor,
        loud_db: loud,
    };

    // A recording with no dynamics is a room, not a sentence — whether it
    // is a quiet room or a loud one.
    if loud - floor < cfg.min_dynamic_range_db {
        return empty;
    }

    let enter = (floor + cfg.enter_margin_db).max(cfg.absolute_floor_db);
    let exit = (floor + cfg.exit_margin_db).max(cfg.absolute_floor_db);
    let hangover_frames = cfg.hangover_ms.div_ceil(cfg.hop_ms.max(1));

    // Raw bursts, before padding: a frame is speech if it is loud, or if it
    // is moderately loud and looks like a fricative.
    let mut bursts: Vec<Speech> = Vec::new();
    let mut open: Option<Speech> = None;
    let mut quiet_run = 0usize;
    for (i, &level) in frames.db.iter().enumerate() {
        let start = i * frames.hop;
        let end = (start + frames.frame).min(samples.len());
        let fricative = level >= exit && frames.zcr[i] >= 0.15;
        let speechy = match open {
            Some(_) => level >= exit || fricative,
            None => level >= enter || (fricative && level >= enter - cfg.exit_margin_db),
        };
        match (&mut open, speechy) {
            (None, true) => open = Some(Speech { start, end }),
            (Some(seg), true) => {
                seg.end = end;
                quiet_run = 0;
            }
            (Some(_), false) => {
                quiet_run += 1;
                if quiet_run > hangover_frames {
                    bursts.push(open.take().expect("segment is open"));
                    quiet_run = 0;
                }
            }
            (None, false) => {}
        }
    }
    if let Some(seg) = open.take() {
        bursts.push(seg);
    }

    // Clicks are dropped on their unpadded length, so padding cannot
    // promote a key press into an utterance.
    let min_speech = hz as usize * cfg.min_speech_ms / 1000;
    bursts.retain(|seg| seg.len() >= min_speech);
    if bursts.is_empty() {
        return empty;
    }

    let lead = hz as usize * cfg.lead_pad_ms / 1000;
    let tail = hz as usize * cfg.hangover_ms / 1000;
    let merge_gap = hz as usize * cfg.merge_gap_ms / 1000;
    let mut segments: Vec<Speech> = Vec::new();
    for burst in bursts {
        let padded = Speech {
            start: burst.start.saturating_sub(lead),
            end: (burst.end + tail).min(samples.len()),
        };
        match segments.last_mut() {
            Some(prev) if padded.start <= prev.end + merge_gap => {
                prev.end = prev.end.max(padded.end);
            }
            _ => segments.push(padded),
        }
    }

    let speech_samples = segments.iter().map(|s| s.len()).sum();
    if speech_samples < hz as usize * cfg.min_total_speech_ms / 1000 {
        return empty;
    }

    VadResult {
        segments,
        speech_samples,
        noise_floor_db: floor,
        loud_db: loud,
    }
}

/// Cuts the lead-in and the tail off `samples`, keeping everything between
/// the first and last speech. Returns `None` when there is no speech to
/// keep — the caller reports that to the user rather than sending a
/// recording of a room to the recogniser.
pub fn trim_to_speech(samples: &[f32], hz: u32, cfg: &VadConfig) -> Option<(Vec<f32>, VadResult)> {
    let result = detect(samples, hz, cfg);
    let span = result.span()?;
    Some((samples[span].to_vec(), result))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HZ: u32 = 16_000;

    /// A reproducible noise source: the tests must not depend on a seed
    /// chosen by the operating system.
    struct Noise(u32);

    impl Noise {
        fn next(&mut self) -> f32 {
            // Numerical Recipes LCG; any full-period generator would do.
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (self.0 >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0
        }
    }

    fn ms(n: usize) -> usize {
        HZ as usize * n / 1000
    }

    /// Room tone at `level`, with a voiced burst of `speech` amplitude
    /// between `from` and `to` milliseconds.
    fn utterance(total_ms: usize, level: f32, speech: f32, from: usize, to: usize) -> Vec<f32> {
        let mut noise = Noise(7);
        (0..ms(total_ms))
            .map(|i| {
                let t = i as f32 / HZ as f32;
                let room = noise.next() * level;
                if i >= ms(from) && i < ms(to) {
                    room + (t * 2.0 * std::f32::consts::PI * 140.0).sin() * speech
                } else {
                    room
                }
            })
            .collect()
    }

    #[test]
    fn a_quiet_speaker_is_found_where_the_old_peak_rule_reported_silence() {
        // Peak well under the 0.005 the recorder used to demand.
        let samples = utterance(2000, 0.00005, 0.003, 700, 1300);
        let result = detect(&samples, HZ, &VadConfig::default());
        assert!(result.has_speech(), "{result:?}");
        let span = result.span().unwrap();
        assert!(span.start <= ms(700) && span.start >= ms(700) - ms(200));
        assert!(span.end >= ms(1300) && span.end <= ms(1300) + ms(300));
    }

    #[test]
    fn a_loud_room_with_nothing_said_is_not_speech() {
        // Six times louder than the old 0.01 threshold, and entirely flat:
        // the amplitude rule kept all of this, the dynamics rule keeps none.
        let mut noise = Noise(11);
        let samples: Vec<f32> = (0..ms(2000)).map(|_| noise.next() * 0.06).collect();
        let result = detect(&samples, HZ, &VadConfig::default());
        assert!(!result.has_speech(), "{result:?}");
    }

    #[test]
    fn a_single_click_is_not_an_utterance() {
        let mut noise = Noise(3);
        let mut samples: Vec<f32> = (0..ms(1500)).map(|_| noise.next() * 0.0005).collect();
        for sample in samples[ms(600)..ms(615)].iter_mut() {
            *sample = 0.6;
        }
        let result = detect(&samples, HZ, &VadConfig::default());
        assert!(
            !result.has_speech(),
            "a 15 ms transient is not speech: {result:?}"
        );
    }

    #[test]
    fn a_pause_inside_one_utterance_does_not_split_the_audio() {
        let mut samples = utterance(3000, 0.0005, 0.05, 500, 900);
        let second = utterance(3000, 0.0, 0.05, 1100, 1800);
        for (a, b) in samples.iter_mut().zip(second) {
            *a += b;
        }
        let result = detect(&samples, HZ, &VadConfig::default());
        let span = result.span().unwrap();
        // Whatever the segmentation, the kept audio spans both halves and
        // the 200 ms gap between them.
        assert!(span.start <= ms(500));
        assert!(span.end >= ms(1800));
        assert!(span.end < ms(2400), "the trailing silence is still cut");
    }

    #[test]
    fn a_dc_offset_does_not_hide_the_speech() {
        let clean = utterance(2000, 0.0002, 0.02, 800, 1400);
        let offset: Vec<f32> = clean.iter().map(|s| s + 0.25).collect();
        assert!(!detect(&offset, HZ, &VadConfig::default()).has_speech());
        let fixed = remove_dc(&offset);
        assert!(detect(&fixed, HZ, &VadConfig::default()).has_speech());
    }

    #[test]
    fn a_recording_of_nothing_at_all_reports_no_speech() {
        assert!(!detect(&[], HZ, &VadConfig::default()).has_speech());
        assert!(!detect(&vec![0.0; ms(1000)], HZ, &VadConfig::default()).has_speech());
    }

    #[test]
    fn trimming_keeps_the_speech_and_drops_the_rest() {
        let samples = utterance(4000, 0.0003, 0.04, 1500, 2200);
        let (trimmed, result) = trim_to_speech(&samples, HZ, &VadConfig::default()).unwrap();
        assert!(trimmed.len() < samples.len() / 2);
        assert!(trimmed.len() >= ms(700));
        assert!(result.snr_db() > 20.0, "{result:?}");
    }

    #[test]
    fn the_threshold_is_relative_so_gain_does_not_change_the_verdict() {
        let quiet = utterance(2500, 0.0004, 0.01, 900, 1600);
        let loud: Vec<f32> = quiet.iter().map(|s| s * 40.0).collect();
        let a = detect(&quiet, HZ, &VadConfig::default());
        let b = detect(&loud, HZ, &VadConfig::default());
        assert!(a.has_speech() && b.has_speech());
        // Same decision, shifted by exactly the gain.
        assert_eq!(a.segments, b.segments);
        assert!((b.noise_floor_db - a.noise_floor_db - 32.0).abs() < 1.0);
    }
}
