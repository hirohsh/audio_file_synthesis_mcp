use std::fs::{self, File};
use std::path::Path;

use claxon::FlacReader;
use minimp3::{Decoder as Mp3Decoder, Error as Mp3Error};

use crate::error::AppError;

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
}

pub fn decode_audio(path: &Path) -> Result<DecodedAudio, AppError> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());

    match extension.as_deref() {
        Some("wav") | Some("wave") => decode_wav(path),
        Some("mp3") => decode_mp3(path),
        Some("flac") => decode_flac(path),
        Some(other) => Err(AppError::UnsupportedFormat(format!(
            "unsupported extension: {other}"
        ))),
        None => Err(AppError::UnsupportedFormat(
            "input file has no extension".to_string(),
        )),
    }
}

fn decode_mp3(path: &Path) -> Result<DecodedAudio, AppError> {
    let file = File::open(path).map_err(|source| AppError::io_with_path(path, source))?;
    let mut decoder = Mp3Decoder::new(file);

    let mut sample_rate: Option<u32> = None;
    let mut channels: Option<u16> = None;
    let mut samples: Vec<f32> = Vec::new();

    loop {
        match decoder.next_frame() {
            Ok(frame) => {
                if frame.sample_rate <= 0 || frame.channels == 0 {
                    return Err(AppError::Decode(format!(
                        "MP3 '{}' had invalid frame metadata",
                        path.display()
                    )));
                }

                let frame_rate = frame.sample_rate as u32;
                let frame_channels = u16::try_from(frame.channels).map_err(|_| {
                    AppError::Decode(format!(
                        "MP3 '{}' has too many channels ({})",
                        path.display(),
                        frame.channels
                    ))
                })?;

                if let Some(rate) = sample_rate {
                    if rate != frame_rate {
                        return Err(AppError::Decode(format!(
                            "MP3 '{}' uses varying sample rates",
                            path.display()
                        )));
                    }
                } else {
                    sample_rate = Some(frame_rate);
                }

                if let Some(channel_count) = channels {
                    if channel_count != frame_channels {
                        return Err(AppError::Decode(format!(
                            "MP3 '{}' uses varying channel counts",
                            path.display()
                        )));
                    }
                } else {
                    channels = Some(frame_channels);
                }

                samples.extend(frame.data.into_iter().map(|sample| sample as f32 / 32768.0));
            }
            Err(Mp3Error::Eof) => break,
            Err(error) => {
                return Err(AppError::Decode(format!(
                    "failed to decode MP3 '{}': {error}",
                    path.display()
                )));
            }
        }
    }

    if samples.is_empty() {
        return Err(AppError::Decode(format!(
            "MP3 '{}' contained no decodable frames",
            path.display()
        )));
    }

    Ok(DecodedAudio {
        samples,
        channels: channels.expect("channels set when samples are present"),
        sample_rate: sample_rate.expect("sample_rate set when samples are present"),
    })
}

fn decode_flac(path: &Path) -> Result<DecodedAudio, AppError> {
    let mut reader = FlacReader::open(path).map_err(|error| {
        AppError::Decode(format!("failed to open FLAC '{}': {error}", path.display()))
    })?;
    let info = reader.streaminfo();

    let channels = info.channels as u16;
    if channels == 0 || info.sample_rate == 0 {
        return Err(AppError::Decode(format!(
            "FLAC '{}' has invalid stream info",
            path.display()
        )));
    }

    let bits_per_sample = info.bits_per_sample;
    if bits_per_sample == 0 || bits_per_sample > 32 {
        return Err(AppError::UnsupportedFormat(format!(
            "FLAC '{}' bits_per_sample={} is unsupported",
            path.display(),
            bits_per_sample
        )));
    }
    let scale = if bits_per_sample == 1 {
        1.0_f32
    } else {
        (1_u64 << (bits_per_sample - 1)) as f32
    };

    let mut samples = Vec::new();
    for sample in reader.samples() {
        let sample = sample.map_err(|error| {
            AppError::Decode(format!(
                "failed to decode FLAC sample from '{}': {error}",
                path.display()
            ))
        })?;
        samples.push(sample as f32 / scale);
    }

    if samples.is_empty() {
        return Err(AppError::Decode(format!(
            "FLAC '{}' contained no samples",
            path.display()
        )));
    }
    if samples.len() % channels as usize != 0 {
        return Err(AppError::Decode(format!(
            "FLAC '{}' sample count is not divisible by channels",
            path.display()
        )));
    }

    Ok(DecodedAudio {
        samples,
        channels,
        sample_rate: info.sample_rate,
    })
}

fn decode_wav(path: &Path) -> Result<DecodedAudio, AppError> {
    let bytes = fs::read(path).map_err(|source| AppError::io_with_path(path, source))?;
    if bytes.len() < 12 {
        return Err(AppError::Decode(
            "file is too small for WAV header".to_string(),
        ));
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(AppError::Decode("invalid WAV RIFF/WAVE header".to_string()));
    }

    let mut cursor = 12usize;
    let mut format: Option<WavFormat> = None;
    let mut data_chunk: Option<(usize, usize)> = None;
    while cursor + 8 <= bytes.len() {
        let chunk_id = &bytes[cursor..cursor + 4];
        let chunk_size = read_u32_le(&bytes, cursor + 4)? as usize;
        let chunk_start = cursor + 8;
        let chunk_end = chunk_start
            .checked_add(chunk_size)
            .ok_or_else(|| AppError::Decode("WAV chunk size overflow".to_string()))?;
        if chunk_end > bytes.len() {
            return Err(AppError::Decode(
                "WAV chunk length exceeds file size".to_string(),
            ));
        }

        if chunk_id == b"fmt " {
            format = Some(parse_fmt_chunk(&bytes[chunk_start..chunk_end])?);
        } else if chunk_id == b"data" {
            data_chunk = Some((chunk_start, chunk_end));
        }

        let padded = chunk_size % 2;
        cursor = chunk_end + padded;
    }

    let format = format.ok_or_else(|| AppError::Decode("missing fmt chunk".to_string()))?;
    let (data_start, data_end) =
        data_chunk.ok_or_else(|| AppError::Decode("missing data chunk".to_string()))?;
    decode_pcm_data(&bytes[data_start..data_end], format)
}

#[derive(Copy, Clone, Debug)]
struct WavFormat {
    audio_format: u16,
    channels: u16,
    sample_rate: u32,
    bits_per_sample: u16,
}

fn parse_fmt_chunk(bytes: &[u8]) -> Result<WavFormat, AppError> {
    if bytes.len() < 16 {
        return Err(AppError::Decode(
            "fmt chunk is shorter than 16 bytes".to_string(),
        ));
    }
    let audio_format = read_u16_le(bytes, 0)?;
    let channels = read_u16_le(bytes, 2)?;
    let sample_rate = read_u32_le(bytes, 4)?;
    let bits_per_sample = read_u16_le(bytes, 14)?;
    if channels == 0 || sample_rate == 0 {
        return Err(AppError::Decode(
            "invalid fmt chunk: channels/sample_rate must be > 0".to_string(),
        ));
    }
    Ok(WavFormat {
        audio_format,
        channels,
        sample_rate,
        bits_per_sample,
    })
}

fn decode_pcm_data(bytes: &[u8], format: WavFormat) -> Result<DecodedAudio, AppError> {
    let bytes_per_sample = match (format.audio_format, format.bits_per_sample) {
        (1, 8) => 1,
        (1, 16) => 2,
        (1, 24) => 3,
        (1, 32) => 4,
        (3, 32) => 4,
        _ => {
            return Err(AppError::UnsupportedFormat(format!(
                "WAV format {} with {} bits per sample is not supported",
                format.audio_format, format.bits_per_sample
            )));
        }
    };

    if bytes.len() % bytes_per_sample != 0 {
        return Err(AppError::Decode(
            "WAV data chunk is not aligned to sample size".to_string(),
        ));
    }
    let sample_count = bytes.len() / bytes_per_sample;
    if sample_count % format.channels as usize != 0 {
        return Err(AppError::Decode(
            "WAV data sample count is not aligned to channels".to_string(),
        ));
    }

    let mut samples = Vec::with_capacity(sample_count);
    match (format.audio_format, format.bits_per_sample) {
        (1, 8) => {
            for &value in bytes {
                samples.push((value as f32 - 128.0) / 128.0);
            }
        }
        (1, 16) => {
            for chunk in bytes.chunks_exact(2) {
                let value = i16::from_le_bytes([chunk[0], chunk[1]]);
                samples.push(value as f32 / 32768.0);
            }
        }
        (1, 24) => {
            for chunk in bytes.chunks_exact(3) {
                let raw = (chunk[0] as i32) | ((chunk[1] as i32) << 8) | ((chunk[2] as i32) << 16);
                let signed = if (raw & 0x0080_0000) != 0 {
                    raw | !0x00FF_FFFF
                } else {
                    raw
                };
                samples.push(signed as f32 / 8_388_608.0);
            }
        }
        (1, 32) => {
            for chunk in bytes.chunks_exact(4) {
                let value = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                samples.push(value as f32 / 2_147_483_648.0);
            }
        }
        (3, 32) => {
            for chunk in bytes.chunks_exact(4) {
                let value = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                if !value.is_finite() {
                    return Err(AppError::Decode(
                        "WAV float sample contains non-finite value".to_string(),
                    ));
                }
                samples.push(value);
            }
        }
        _ => unreachable!("validated earlier"),
    }

    Ok(DecodedAudio {
        samples,
        channels: format.channels,
        sample_rate: format.sample_rate,
    })
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Result<u16, AppError> {
    let slice = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| AppError::Decode("unexpected EOF while reading u16".to_string()))?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Result<u32, AppError> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| AppError::Decode("unexpected EOF while reading u32".to_string()))?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn unique_temp_dir() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time moved backwards")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "audio_decode_test_{}_{}",
            std::process::id(),
            nanos
        ))
    }

    #[test]
    fn invalid_mp3_is_not_placeholder_error() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let path = temp_dir.join("broken.mp3");
        fs::write(&path, b"invalid-mp3").expect("write file");

        let error = decode_audio(&path).expect_err("decode should fail");
        assert!(matches!(error, AppError::Decode(_)));
        assert!(!error.to_string().contains("not implemented"));

        fs::remove_dir_all(&temp_dir).expect("cleanup temp dir");
    }

    #[test]
    fn invalid_flac_is_not_placeholder_error() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let path = temp_dir.join("broken.flac");
        fs::write(&path, b"invalid-flac").expect("write file");

        let error = decode_audio(&path).expect_err("decode should fail");
        assert!(matches!(error, AppError::Decode(_)));
        assert!(!error.to_string().contains("not implemented"));

        fs::remove_dir_all(&temp_dir).expect("cleanup temp dir");
    }
}
