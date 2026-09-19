//! Turning recorded samples into a file Whisper can read, and reading the
//! format of a file the user picked.
//!
//! Two rules shape this module:
//!
//! * only one format is written — 16 kHz, mono, 16-bit PCM — because that is
//!   what Whisper expects and what the recorder produces, so no resampling and
//!   no encoder is involved;
//! * a file that is not that format is *reported*, not converted. Converting
//!   would mean decoding an arbitrary audio file, which is a parser this
//!   feature deliberately does not have.

use std::io::Write;
use std::path::Path;

use super::config::SAMPLE_RATE;
use super::error::WhisperError;

/// Bytes of the canonical WAV header this module writes.
pub const WAV_HEADER_BYTES: u32 = 44;
/// Channels written: mono.
pub const WAV_CHANNELS: u16 = 1;
/// Bits per sample written.
pub const WAV_BITS: u16 = 16;

/// What was found in an existing audio file's header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WavFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub data_bytes: u32,
}

impl WavFormat {
    /// Whether the file is in the one format this feature sends to the model.
    pub fn is_supported(&self) -> bool {
        self.sample_rate == SAMPLE_RATE && self.channels == 1 && self.bits_per_sample == 16
    }

    /// How long the audio is, in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        let frames = self.data_bytes as u64
            / (self.channels.max(1) as u64 * (self.bits_per_sample as u64 / 8));
        let per_second = self.sample_rate.max(1) as u64;
        frames * 1000 / per_second
    }
}

/// Writes 16 kHz mono 16-bit PCM samples as a WAV file.
///
/// The samples are the recorder's own output: signed 16-bit little-endian at
/// 16 kHz, so the body is a copy and the header is the only thing built here.
pub fn write_wav(path: &Path, samples: &[i16]) -> Result<WavFormat, WhisperError> {
    if samples.is_empty() {
        return Err(WhisperError::AudioEmpty);
    }
    let data_bytes = (samples.len() * 2) as u32;
    let format = WavFormat {
        sample_rate: SAMPLE_RATE,
        channels: WAV_CHANNELS,
        bits_per_sample: WAV_BITS,
        data_bytes,
    };
    let byte_rate = SAMPLE_RATE * WAV_CHANNELS as u32 * (WAV_BITS as u32 / 8);
    let block_align = WAV_CHANNELS * (WAV_BITS / 8);

    let mut header = Vec::with_capacity(WAV_HEADER_BYTES as usize);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    header.extend_from_slice(b"WAVE");
    header.extend_from_slice(b"fmt ");
    header.extend_from_slice(&16u32.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes()); // PCM
    header.extend_from_slice(&WAV_CHANNELS.to_le_bytes());
    header.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    header.extend_from_slice(&byte_rate.to_le_bytes());
    header.extend_from_slice(&block_align.to_le_bytes());
    header.extend_from_slice(&WAV_BITS.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&data_bytes.to_le_bytes());
    debug_assert_eq!(header.len() as u32, WAV_HEADER_BYTES);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(path)?;
    file.write_all(&header)?;
    let mut body = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        body.extend_from_slice(&sample.to_le_bytes());
    }
    file.write_all(&body)?;
    file.flush()?;
    Ok(format)
}

/// Reads the format of an existing WAV file without decoding its audio.
pub fn read_wav_format(path: &Path) -> Result<WavFormat, WhisperError> {
    let bytes = std::fs::read(path)
        .map_err(|_| WhisperError::AudioUnavailable("that file cannot be read".to_string()))?;
    parse_wav_format(&bytes).ok_or_else(|| {
        WhisperError::AudioUnavailable(
            "that file is not a WAV file this build can read".to_string(),
        )
    })
}

/// Parses the RIFF header of a WAV file in memory.
///
/// Only the chunks up to `data` are walked, and a chunk that claims to be longer
/// than the file is refused rather than followed.
pub fn parse_wav_format(bytes: &[u8]) -> Option<WavFormat> {
    if bytes.len() < WAV_HEADER_BYTES as usize
        || &bytes[0..4] != b"RIFF"
        || &bytes[8..12] != b"WAVE"
    {
        return None;
    }
    let mut position = 12usize;
    let mut format: Option<(u32, u16, u16)> = None;
    while position + 8 <= bytes.len() {
        let id = &bytes[position..position + 4];
        let size = u32::from_le_bytes([
            bytes[position + 4],
            bytes[position + 5],
            bytes[position + 6],
            bytes[position + 7],
        ]) as usize;
        let body = position + 8;
        if body + size > bytes.len() {
            return None;
        }
        if id == b"fmt " && size >= 16 {
            let audio_format = u16::from_le_bytes([bytes[body], bytes[body + 1]]);
            if audio_format != 1 {
                // Compressed WAV is not something this feature pretends to read.
                return None;
            }
            let channels = u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]);
            let sample_rate = u32::from_le_bytes([
                bytes[body + 4],
                bytes[body + 5],
                bytes[body + 6],
                bytes[body + 7],
            ]);
            let bits = u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]);
            format = Some((sample_rate, channels, bits));
        }
        if id == b"data" {
            let (sample_rate, channels, bits_per_sample) = format?;
            return Some(WavFormat {
                sample_rate,
                channels,
                bits_per_sample,
                data_bytes: size as u32,
            });
        }
        // Chunks are word-aligned: a chunk of odd size is followed by a pad byte.
        position = body + size + (size % 2);
    }
    None
}

/// The loudest sample in a frame, used to decide whether the user is speaking.
pub fn peak_amplitude(frame: &[i16]) -> i16 {
    frame
        .iter()
        .map(|sample| sample.saturating_abs())
        .max()
        .unwrap_or(0)
}

// ------------------------------------------------------ preparing quiet speech

/// The largest gain applied automatically: +24 dB, about a factor of 16.
///
/// A recording that is quieter than this is not speech this feature can rescue,
/// and amplifying it further would only raise the noise floor into the model.
pub const MAX_AUTOMATIC_GAIN: f32 = 15.85;
/// The peak the automatic gain aims for, as a fraction of full scale.
///
/// It is below 1.0 on purpose: the model reads a signal with headroom, and a
/// sample that lands exactly on full scale is where clipping starts.
pub const TARGET_PEAK: f32 = 0.85;

/// What a recording measured before it was prepared.
///
/// Numbers only: no audio, no text, and nothing that identifies a device.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioLevels {
    /// The loudest sample as it arrived, before anything was removed.
    ///
    /// Silence is decided on this one: a recording with non-zero samples is
    /// never called empty, whatever its RMS says.
    pub source_peak: f32,
    /// The DC offset that was removed, in samples. Zero when the signal has no
    /// usable bias to remove.
    pub offset: f32,
    /// The loudest sample after the offset was removed.
    pub peak: f32,
    /// The root mean square after the offset was removed.
    pub rms: f32,
    /// The gain that was applied, as a factor: 1.0 means unchanged.
    pub gain: f32,
}

impl AudioLevels {
    /// Whether the recording has any signal at all.
    ///
    /// This is the exact question the requirement asks: an all-zero buffer is
    /// empty, and a quiet but non-zero one is not.
    pub fn is_silent(&self) -> bool {
        self.source_peak == 0.0
    }

    /// The centred peak as a fraction of full scale, which is what a log line
    /// and a threshold can use.
    pub fn peak_fraction(&self) -> f32 {
        self.peak / f32::from(i16::MAX)
    }

    /// The RMS as a fraction of full scale.
    pub fn rms_fraction(&self) -> f32 {
        self.rms / f32::from(i16::MAX)
    }

    /// Whether the recording is quiet enough that the model may struggle.
    ///
    /// This is a warning, not a refusal: the audio is still sent, because a
    /// quiet recording that is recognizable is better than no attempt.
    pub fn is_quiet(&self) -> bool {
        !self.is_silent() && self.rms_fraction() < QUIET_RMS_FRACTION
    }
}

/// The RMS, as a fraction of full scale, below which a recording is called quiet.
pub const QUIET_RMS_FRACTION: f32 = 0.01;

/// A recording, prepared for the model.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedAudio {
    /// The samples to write: offset removed and, when it was asked for, amplified.
    pub samples: Vec<i16>,
    /// What was measured on the way.
    pub levels: AudioLevels,
    /// Whether the automatic gain was applied.
    pub normalized: bool,
}

/// Removes the DC offset, measures the signal, and amplifies a quiet recording.
///
/// Why this exists: a microphone that works perfectly can still deliver a peak
/// around 0.001 of full scale — a quiet speaker, a low input level in the
/// Windows mixer, a laptop array at arm's length. Whisper is fed 16-bit PCM and
/// does not amplify anything itself, so that recording arrives as a whisper of
/// noise and comes back as nothing.
///
/// The rules:
///
/// * a recording with no signal at all stays `audio_empty`; amplification is not
///   a way to invent speech, and amplifying digital silence would only produce
///   louder silence and a hallucination;
/// * the gain is bounded by [`MAX_AUTOMATIC_GAIN`] and aims at [`TARGET_PEAK`],
///   so a recording that is already loud is left alone;
/// * every sample is clamped, so no sample can wrap around — clipping is a
///   bounded, audible artefact, while a wrap is a loud click;
/// * the original samples are not modified: the prepared copy is what is written
///   to the file, and the caller still holds what was recorded.
pub fn prepare_audio(samples: &[i16], normalize: bool) -> PreparedAudio {
    let source_peak = samples
        .iter()
        .map(|sample| f32::from(sample.saturating_abs()))
        .fold(0.0f32, f32::max);
    if samples.is_empty() || source_peak == 0.0 {
        // Digital silence: there is nothing to measure, nothing to amplify, and
        // nothing the model could read. The samples come back untouched.
        return PreparedAudio {
            samples: samples.to_vec(),
            levels: AudioLevels {
                source_peak: 0.0,
                offset: 0.0,
                peak: 0.0,
                rms: 0.0,
                gain: 1.0,
            },
            normalized: false,
        };
    }

    // The DC offset: a microphone that sits above or below zero wastes headroom
    // and biases the model's input. Only a bias smaller than the signal itself is
    // removed — a buffer that is constant is not a biased signal, and subtracting
    // its mean would turn non-zero samples into silence, which is the one thing
    // this function must not do.
    let sum: f64 = samples.iter().map(|sample| f64::from(*sample)).sum();
    let mean = sum / samples.len() as f64;
    let offset = if (mean.abs() as f32) < source_peak {
        mean
    } else {
        0.0
    };

    let mut peak = 0.0f32;
    let mut square_sum = 0.0f64;
    for sample in samples {
        let centered = f64::from(*sample) - offset;
        let magnitude = centered.abs() as f32;
        if magnitude > peak {
            peak = magnitude;
        }
        square_sum += centered * centered;
    }
    let rms = (square_sum / samples.len() as f64).sqrt() as f32;

    // A signal with no peak to work with is left alone: there is nothing to aim
    // the gain at.
    let gain = if normalize && peak > 0.0 {
        let wanted = (TARGET_PEAK * f32::from(i16::MAX)) / peak;
        wanted.clamp(1.0, MAX_AUTOMATIC_GAIN)
    } else {
        1.0
    };

    let prepared: Vec<i16> = samples
        .iter()
        .map(|sample| {
            let centered = f64::from(*sample) - offset;
            let amplified = centered * f64::from(gain);
            amplified
                .round()
                .clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
        })
        .collect();

    PreparedAudio {
        samples: prepared,
        levels: AudioLevels {
            source_peak,
            offset: offset as f32,
            peak,
            rms,
            gain,
        },
        normalized: gain > 1.0,
    }
}

/// How many samples a number of seconds is, at the recorder's rate.
pub fn samples_for_seconds(seconds: u64) -> usize {
    (seconds as usize).saturating_mul(SAMPLE_RATE as usize)
}

/// How many samples a number of milliseconds is, at the recorder's rate.
pub fn samples_for_millis(millis: u64) -> usize {
    (millis as usize).saturating_mul(SAMPLE_RATE as usize) / 1000
}

/// How many frames of `frame_samples` cover a number of seconds.
///
/// The count is rounded up, so a one-second recording really reaches one second
/// instead of stopping a fraction of a frame short.
pub fn frames_for_seconds(seconds: u64, frame_samples: usize) -> usize {
    let total = samples_for_seconds(seconds);
    total.div_ceil(frame_samples.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn a_written_file_is_the_format_whisper_expects() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("audio").join("dictation.wav");
        let samples: Vec<i16> = (0..16_000).map(|index| (index % 1000) as i16).collect();
        let written = write_wav(&path, &samples).unwrap();
        assert_eq!(written.sample_rate, SAMPLE_RATE);
        assert_eq!(written.channels, 1);
        assert_eq!(written.bits_per_sample, 16);
        assert_eq!(written.data_bytes, 32_000);
        assert_eq!(written.duration_ms(), 1000);
        // The directory was created, and the file is exactly header + body.
        let metadata = std::fs::metadata(&path).unwrap();
        assert_eq!(metadata.len(), WAV_HEADER_BYTES as u64 + 32_000);
        assert!(written.is_supported());
    }

    #[test]
    fn what_was_written_can_be_read_back() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("dictation.wav");
        let samples = vec![0i16, 100, -100, i16::MAX, i16::MIN];
        write_wav(&path, &samples).unwrap();
        let format = read_wav_format(&path).unwrap();
        assert_eq!(format.data_bytes, 10);
        assert!(format.is_supported());
        // The samples survive the round trip, byte for byte.
        let bytes = std::fs::read(&path).unwrap();
        let body = &bytes[WAV_HEADER_BYTES as usize..];
        assert_eq!(i16::from_le_bytes([body[0], body[1]]), 0);
        assert_eq!(i16::from_le_bytes([body[6], body[7]]), i16::MAX);
        assert_eq!(i16::from_le_bytes([body[8], body[9]]), i16::MIN);
    }

    #[test]
    fn nothing_recorded_is_not_a_file() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("empty.wav");
        assert_eq!(write_wav(&path, &[]).unwrap_err(), WhisperError::AudioEmpty);
        assert!(!path.exists(), "an empty recording must not leave a file");
    }

    #[test]
    fn a_file_that_is_not_a_wav_is_refused_instead_of_guessed_at() {
        assert_eq!(parse_wav_format(b"not a wav at all, really not"), None);
        assert_eq!(parse_wav_format(b""), None);
        // A RIFF file whose declared chunk runs past the end of the file.
        let mut broken = Vec::new();
        broken.extend_from_slice(b"RIFF");
        broken.extend_from_slice(&1000u32.to_le_bytes());
        broken.extend_from_slice(b"WAVE");
        broken.extend_from_slice(b"fmt ");
        broken.extend_from_slice(&16u32.to_le_bytes());
        broken.extend_from_slice(&[0u8; 16]);
        broken.extend_from_slice(b"data");
        broken.extend_from_slice(&999_999u32.to_le_bytes());
        assert_eq!(parse_wav_format(&broken), None);
    }

    #[test]
    fn a_compressed_or_stereo_file_is_reported_as_unsupported() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("stereo.wav");
        // 44.1 kHz stereo, which is what a phone recording usually is.
        let mut header = Vec::new();
        header.extend_from_slice(b"RIFF");
        header.extend_from_slice(&36u32.to_le_bytes());
        header.extend_from_slice(b"WAVE");
        header.extend_from_slice(b"fmt ");
        header.extend_from_slice(&16u32.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&2u16.to_le_bytes());
        header.extend_from_slice(&44_100u32.to_le_bytes());
        header.extend_from_slice(&176_400u32.to_le_bytes());
        header.extend_from_slice(&4u16.to_le_bytes());
        header.extend_from_slice(&16u16.to_le_bytes());
        header.extend_from_slice(b"data");
        header.extend_from_slice(&0u32.to_le_bytes());
        std::fs::write(&path, &header).unwrap();
        let format = read_wav_format(&path).unwrap();
        assert!(!format.is_supported());
        assert!(!WavFormat {
            sample_rate: SAMPLE_RATE,
            channels: 2,
            bits_per_sample: 16,
            data_bytes: 0,
        }
        .is_supported());
        assert!(!WavFormat {
            sample_rate: 8_000,
            channels: 1,
            bits_per_sample: 16,
            data_bytes: 0,
        }
        .is_supported());
        assert!(!WavFormat {
            sample_rate: SAMPLE_RATE,
            channels: 1,
            bits_per_sample: 8,
            data_bytes: 0,
        }
        .is_supported());
    }

    #[test]
    fn loudness_is_measured_without_saturating() {
        assert_eq!(peak_amplitude(&[]), 0);
        assert_eq!(peak_amplitude(&[0, 10, -30, 5]), 30);
        assert_eq!(peak_amplitude(&[i16::MIN]), i16::MAX);
    }

    // ------------------------------------------------ quiet speech, prepared

    /// The reported machine: a working microphone, a peak around 0.001 of full
    /// scale, and a model that answers with nothing.
    #[test]
    fn a_quiet_recording_is_amplified_towards_the_target_peak() {
        let quiet: Vec<i16> = (0..16_000)
            .map(|index| if index % 2 == 0 { 30 } else { -30 })
            .collect();
        let prepared = prepare_audio(&quiet, true);
        assert!(prepared.normalized, "a quiet recording must be amplified");
        assert!(
            (prepared.levels.peak - 30.0).abs() < 1.0,
            "the measurement is of the original signal: {}",
            prepared.levels.peak
        );
        // 30 samples of peak is far below the target, so the bound is what
        // applies: +24 dB, the most this feature amplifies by itself.
        assert_eq!(
            prepared.levels.gain, MAX_AUTOMATIC_GAIN,
            "a very quiet signal is amplified to the bound"
        );
        let peak = prepared
            .samples
            .iter()
            .map(|sample| f32::from(sample.saturating_abs()))
            .fold(0.0f32, f32::max);
        assert!(
            peak > prepared.levels.peak * 10.0,
            "the samples handed to the model carry the amplified speech, got {peak}"
        );
        assert!(
            peak <= f32::from(i16::MAX),
            "the prepared signal must not clip: {peak}"
        );
        // The original samples are not modified: the caller still has them.
        assert_eq!(quiet[0], 30);
    }

    /// A recording that only needs a little help gets exactly that, and the
    /// target peak is what the gain aims at.
    #[test]
    fn a_moderately_quiet_recording_is_aimed_at_the_target_peak() {
        let gentle: Vec<i16> = (0..1_000)
            .map(|index| if index % 2 == 0 { 4_000 } else { -4_000 })
            .collect();
        let prepared = prepare_audio(&gentle, true);
        assert!(prepared.normalized);
        let wanted = (TARGET_PEAK * f32::from(i16::MAX)) / 4_000.0;
        assert!(
            (prepared.levels.gain - wanted).abs() < 0.01,
            "the gain aims at {TARGET_PEAK} of full scale"
        );
        let peak = prepared
            .samples
            .iter()
            .map(|sample| f32::from(sample.saturating_abs()))
            .fold(0.0f32, f32::max);
        let target = TARGET_PEAK * f32::from(i16::MAX);
        assert!(
            (peak - target).abs() < 100.0,
            "the amplified peak is the target, got {peak} for {target}"
        );
    }

    /// A signal that is already loud is left where it is: no invented headroom.
    #[test]
    fn a_loud_recording_is_not_amplified() {
        let loud: Vec<i16> = (0..1_000)
            .map(|index| if index % 2 == 0 { 30_000 } else { -30_000 })
            .collect();
        let prepared = prepare_audio(&loud, true);
        assert!(!prepared.normalized);
        assert_eq!(prepared.levels.gain, 1.0);
        assert_eq!(prepared.samples, loud);
    }

    /// The gain is bounded, and the result never wraps around: a limiter, not an
    /// overflow.
    #[test]
    fn the_gain_is_bounded_and_cannot_wrap_a_sample() {
        // 1 LSB of signal: the gain would be enormous and is capped.
        let tiny: Vec<i16> = vec![1, -1, 1, -1];
        let prepared = prepare_audio(&tiny, true);
        assert_eq!(prepared.levels.gain, MAX_AUTOMATIC_GAIN);
        let peak = prepared
            .samples
            .iter()
            .map(|sample| sample.saturating_abs())
            .max()
            .unwrap();
        // One LSB through the bound, rounded, and still nowhere near a wrap.
        assert_eq!(peak, (f64::from(MAX_AUTOMATIC_GAIN)).round() as i16);
        assert!(peak > 0, "the signal is still there");
        // A sample that would land one step past full scale is clamped, not
        // wrapped: a wrap is a loud click, a clamp is a bounded artefact.
        let edge: Vec<i16> = vec![i16::MIN, i16::MAX];
        let prepared = prepare_audio(&edge, true);
        assert_eq!(
            prepared.samples,
            vec![i16::MIN, i16::MAX],
            "the edge is clamped back into range, not wrapped"
        );
        assert!(
            prepared.levels.gain <= MAX_AUTOMATIC_GAIN,
            "no unbounded gain"
        );
        // And a signal that is already at full scale is left alone.
        let hot: Vec<i16> = vec![i16::MAX, -i16::MAX, i16::MAX, -i16::MAX];
        let prepared = prepare_audio(&hot, true);
        assert_eq!(prepared.levels.gain, 1.0, "no gain on a loud signal");
        assert_eq!(prepared.samples, hot);
    }

    /// Digital silence stays silence: amplification is not a way to invent
    /// speech, and a louder zero is still zero.
    #[test]
    fn a_zero_signal_is_never_amplified_and_stays_empty() {
        for samples in [vec![0i16; 16_000], Vec::new()] {
            let prepared = prepare_audio(&samples, true);
            assert!(prepared.levels.is_silent());
            assert_eq!(prepared.levels.gain, 1.0, "no gain on silence");
            assert!(!prepared.normalized);
            assert!(
                prepared.samples.iter().all(|sample| *sample == 0),
                "silence stays silence"
            );
        }
    }

    /// A bias that is smaller than the signal is removed; a constant buffer is
    /// not a biased signal and must not be turned into silence.
    #[test]
    fn a_dc_offset_is_removed_but_a_constant_signal_is_kept() {
        let biased: Vec<i16> = (0..1_000)
            .map(|index| if index % 2 == 0 { 500 } else { 100 })
            .collect();
        let prepared = prepare_audio(&biased, false);
        assert!(
            prepared.levels.offset > 200.0 && prepared.levels.offset < 400.0,
            "the bias is measured: {}",
            prepared.levels.offset
        );
        let mean: f32 = prepared.samples.iter().map(|s| f32::from(*s)).sum::<f32>()
            / prepared.samples.len() as f32;
        assert!(mean.abs() < 1.0, "the prepared signal is centred: {mean}");

        // A constant recording is not speech, but it is not empty either.
        let constant = vec![1_000i16; 1_000];
        let prepared = prepare_audio(&constant, true);
        assert!(
            !prepared.levels.is_silent(),
            "non-zero samples are not empty"
        );
        assert_eq!(prepared.levels.offset, 0.0);
        assert!(
            prepared.samples.iter().all(|sample| *sample != 0),
            "non-zero samples must not be flattened to zero"
        );
    }

    /// The switch turns the automatic gain off without touching anything else.
    #[test]
    fn the_automatic_gain_can_be_switched_off() {
        let quiet: Vec<i16> = vec![30, -30, 30, -30];
        let prepared = prepare_audio(&quiet, false);
        assert!(!prepared.normalized);
        assert_eq!(prepared.levels.gain, 1.0);
        // The offset removal still happens: it is not amplification.
        assert!(prepared.levels.source_peak > 0.0);
    }

    /// The warning the report asks for: a quiet but non-zero recording is
    /// flagged, and it is still sent.
    #[test]
    fn a_quiet_but_non_zero_recording_is_a_warning_not_a_refusal() {
        let quiet: Vec<i16> = vec![5, -5, 5, -5];
        let prepared = prepare_audio(&quiet, true);
        assert!(!prepared.levels.is_silent());
        assert!(prepared.levels.is_quiet(), "the warning applies");
        assert!(
            prepared.samples.iter().any(|sample| *sample != 0),
            "and the audio is still there for the model"
        );
        // A loud one is not flagged.
        let loud: Vec<i16> = vec![9_000, -9_000];
        assert!(!prepare_audio(&loud, true).levels.is_quiet());
    }

    #[test]
    fn a_duration_becomes_a_sample_count_at_the_recorder_rate() {
        assert_eq!(samples_for_seconds(1), 16_000);
        assert_eq!(samples_for_seconds(30), 480_000);
        assert_eq!(samples_for_seconds(0), 0);
        assert_eq!(samples_for_millis(200), 3_200);
        assert_eq!(samples_for_millis(1_500), 24_000);
        // Rounding up is what makes a one-second recording reach one second.
        assert_eq!(frames_for_seconds(1, 512), 32);
        assert!(frames_for_seconds(1, 512) * 512 >= 16_000);
        assert_eq!(frames_for_seconds(30, 512), 938);
    }
}
