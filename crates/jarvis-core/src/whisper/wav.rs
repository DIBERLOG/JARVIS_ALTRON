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
