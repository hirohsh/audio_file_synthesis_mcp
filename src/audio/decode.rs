use std::fs::{self, File};
use std::path::Path;

use claxon::FlacReader;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::{get_codecs, get_probe};

use crate::error::AppError;

const MAX_FILE_SIZE: u64 = 500 * 1024 * 1024; // 500 MB
const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
const KSDATAFORMAT_SUBTYPE_PCM: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b,
    0x71,
];
const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: [u8; 16] = [
    0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b,
    0x71,
];

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
}

pub fn decode_audio(path: &Path) -> Result<DecodedAudio, AppError> {
    if let Ok(metadata) = std::fs::metadata(path) {
        if metadata.len() > MAX_FILE_SIZE {
            return Err(AppError::Decode(format!(
                "file size {} exceeds maximum of {}",
                metadata.len(),
                MAX_FILE_SIZE
            )));
        }
    } else {
        return Err(AppError::InvalidParams(
            "Failed to read file metadata".to_string(),
        ));
    }

    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());

    match extension.as_deref() {
        Some("wav") | Some("wave") => decode_wav(path),
        Some("mp3") => decode_mp3(path),
        Some("flac") => decode_flac(path),
        Some("m4a") => decode_m4a(path),
        Some(other) => Err(AppError::UnsupportedFormat(format!(
            "unsupported extension: {other}"
        ))),
        None => Err(AppError::UnsupportedFormat(
            "input file has no extension".to_string(),
        )),
    }
}

fn decode_mp3(path: &Path) -> Result<DecodedAudio, AppError> {
    decode_with_symphonia(path, "mp3", "MP3")
}

fn decode_m4a(path: &Path) -> Result<DecodedAudio, AppError> {
    decode_with_symphonia(path, "m4a", "M4A")
}

fn decode_with_symphonia(
    path: &Path,
    hint_extension: &str,
    label: &str,
) -> Result<DecodedAudio, AppError> {
    let file = File::open(path).map_err(|source| AppError::io_with_path(path, source))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    hint.with_extension(hint_extension);

    let probed = get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| {
            AppError::Decode(format!("failed to open {label} '{}': {error}", path.display()))
        })?;
    let mut format = probed.format;

    let (track_id, codec_params) = {
        let track = format
            .default_track()
            .ok_or_else(|| AppError::Decode(format!("{label} '{}' has no default track", path.display())))?;
        (track.id, track.codec_params.clone())
    };

    let mut sample_rate = codec_params.sample_rate;
    let mut channels = codec_params
        .channels
        .and_then(|layout| u16::try_from(layout.count()).ok());

    let mut decoder = get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|error| {
            AppError::Decode(format!(
                "failed to initialize {label} decoder '{}': {error}",
                path.display()
            ))
        })?;

    let mut samples = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(error) => {
                return Err(AppError::Decode(format!(
                    "failed to read {label} packet from '{}': {error}",
                    path.display()
                )));
            }
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = decoder.decode(&packet).map_err(|error| {
            AppError::Decode(format!(
                "failed to decode {label} packet from '{}': {error}",
                path.display()
            ))
        })?;

        let spec = *decoded.spec();
        let packet_channels = u16::try_from(spec.channels.count()).map_err(|_| {
            AppError::Decode(format!(
                "{label} '{}' has too many channels ({})",
                path.display(),
                spec.channels.count()
            ))
        })?;
        if packet_channels == 0 || spec.rate == 0 {
            return Err(AppError::Decode(format!(
                "{label} '{}' had invalid frame metadata",
                path.display()
            )));
        }

        if let Some(rate) = sample_rate {
            if spec.rate != rate {
                return Err(AppError::Decode(format!(
                    "{label} '{}' uses varying sample rates",
                    path.display()
                )));
            }
        } else {
            sample_rate = Some(spec.rate);
        }

        if let Some(channel_count) = channels {
            if packet_channels != channel_count {
                return Err(AppError::Decode(format!(
                    "{label} '{}' uses varying channel counts",
                    path.display()
                )));
            }
        } else {
            channels = Some(packet_channels);
        }

        let mut sample_buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        sample_buffer.copy_interleaved_ref(decoded);
        samples.extend_from_slice(sample_buffer.samples());
    }

    if samples.is_empty() {
        return Err(AppError::Decode(format!(
            "{label} '{}' contained no decodable frames",
            path.display()
        )));
    }
    let channels = channels.ok_or_else(|| {
        AppError::Decode(format!(
            "{label} '{}' is missing channel metadata",
            path.display()
        ))
    })?;
    let sample_rate = sample_rate.ok_or_else(|| {
        AppError::Decode(format!(
            "{label} '{}' is missing sample rate metadata",
            path.display()
        ))
    })?;
    if samples.len() % channels as usize != 0 {
        return Err(AppError::Decode(format!(
            "{label} '{}' sample count is not divisible by channels",
            path.display()
        )));
    }

    Ok(DecodedAudio {
        samples,
        channels,
        sample_rate,
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
    let mut audio_format = read_u16_le(bytes, 0)?;
    let channels = read_u16_le(bytes, 2)?;
    let sample_rate = read_u32_le(bytes, 4)?;
    let bits_per_sample = read_u16_le(bytes, 14)?;
    if channels == 0 || sample_rate == 0 {
        return Err(AppError::Decode(
            "invalid fmt chunk: channels/sample_rate must be > 0".to_string(),
        ));
    }
    if audio_format == WAVE_FORMAT_EXTENSIBLE {
        audio_format = parse_wav_extensible_subformat(bytes)?;
    }
    Ok(WavFormat {
        audio_format,
        channels,
        sample_rate,
        bits_per_sample,
    })
}

fn parse_wav_extensible_subformat(bytes: &[u8]) -> Result<u16, AppError> {
    if bytes.len() < 40 {
        return Err(AppError::Decode(
            "WAV extensible fmt chunk is shorter than 40 bytes".to_string(),
        ));
    }
    let cb_size = read_u16_le(bytes, 16)? as usize;
    if cb_size < 22 {
        return Err(AppError::Decode(
            "WAV extensible fmt chunk has invalid cbSize".to_string(),
        ));
    }

    let subformat = bytes
        .get(24..40)
        .ok_or_else(|| AppError::Decode("WAV extensible fmt chunk is missing SubFormat".to_string()))?;
    if subformat == KSDATAFORMAT_SUBTYPE_PCM {
        Ok(WAVE_FORMAT_PCM)
    } else if subformat == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
        Ok(WAVE_FORMAT_IEEE_FLOAT)
    } else {
        Err(AppError::UnsupportedFormat(
            "WAV extensible subformat is not supported".to_string(),
        ))
    }
}

fn decode_pcm_data(bytes: &[u8], format: WavFormat) -> Result<DecodedAudio, AppError> {
    let bytes_per_sample = match (format.audio_format, format.bits_per_sample) {
        (WAVE_FORMAT_PCM, 8) => 1,
        (WAVE_FORMAT_PCM, 16) => 2,
        (WAVE_FORMAT_PCM, 24) => 3,
        (WAVE_FORMAT_PCM, 32) => 4,
        (WAVE_FORMAT_IEEE_FLOAT, 32) => 4,
        _ => {
            return Err(AppError::UnsupportedFormat(format!(
                "WAV format {} with {} bits per sample is not supported",
                format.audio_format, format.bits_per_sample
            )));
        }
    };

    if !bytes.len().is_multiple_of(bytes_per_sample) {
        return Err(AppError::Decode(
            "WAV data chunk is not aligned to sample size".to_string(),
        ));
    }
    let sample_count = bytes.len() / bytes_per_sample;
    if !sample_count.is_multiple_of(format.channels as usize) {
        return Err(AppError::Decode(
            "WAV data sample count is not aligned to channels".to_string(),
        ));
    }

    let mut samples = Vec::with_capacity(sample_count);
    match (format.audio_format, format.bits_per_sample) {
        (WAVE_FORMAT_PCM, 8) => {
            for &value in bytes {
                samples.push((value as f32 - 128.0) / 128.0);
            }
        }
        (WAVE_FORMAT_PCM, 16) => {
            for chunk in bytes.chunks_exact(2) {
                let value = i16::from_le_bytes([chunk[0], chunk[1]]);
                samples.push(value as f32 / 32768.0);
            }
        }
        (WAVE_FORMAT_PCM, 24) => {
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
        (WAVE_FORMAT_PCM, 32) => {
            for chunk in bytes.chunks_exact(4) {
                let value = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                samples.push(value as f32 / 2_147_483_648.0);
            }
        }
        (WAVE_FORMAT_IEEE_FLOAT, 32) => {
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

    fn wav_extensible_pcm_i16_bytes(
        sample_rate: u32,
        channels: u16,
        samples: &[i16],
        subformat: [u8; 16],
    ) -> Vec<u8> {
        let data_size = (samples.len() * std::mem::size_of::<i16>()) as u32;
        let block_align = channels * std::mem::size_of::<i16>() as u16;
        let byte_rate = sample_rate * block_align as u32;
        let riff_size = 4 + (8 + 40) + (8 + data_size);

        let mut bytes = Vec::with_capacity((riff_size + 8) as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&riff_size.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");

        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&40_u32.to_le_bytes());
        bytes.extend_from_slice(&WAVE_FORMAT_EXTENSIBLE.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&byte_rate.to_le_bytes());
        bytes.extend_from_slice(&block_align.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(&22_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&subformat);

        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_size.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }

        bytes
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

    #[test]
    fn invalid_m4a_is_not_unsupported_extension() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let path = temp_dir.join("broken.m4a");
        fs::write(&path, b"invalid-m4a").expect("write file");

        let error = decode_audio(&path).expect_err("decode should fail");
        assert!(matches!(error, AppError::Decode(_)));
        assert!(!error.to_string().contains("unsupported extension"));

        fs::remove_dir_all(&temp_dir).expect("cleanup temp dir");
    }

    #[test]
    fn wav_extensible_pcm_16_is_supported() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let path = temp_dir.join("extensible.wav");
        let wav_bytes = wav_extensible_pcm_i16_bytes(
            8_000,
            1,
            &[0, 16_384, -16_384],
            KSDATAFORMAT_SUBTYPE_PCM,
        );
        fs::write(&path, wav_bytes).expect("write file");

        let decoded = decode_audio(&path).expect("decode should succeed");
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.sample_rate, 8_000);
        assert_eq!(decoded.samples.len(), 3);
        assert!((decoded.samples[1] - 0.5).abs() < 1e-4);
        assert!((decoded.samples[2] + 0.5).abs() < 1e-4);

        fs::remove_dir_all(&temp_dir).expect("cleanup temp dir");
    }

    #[test]
    fn wav_extensible_with_unsupported_subformat_is_rejected() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let path = temp_dir.join("unsupported_subformat.wav");
        let mut unknown_subformat = KSDATAFORMAT_SUBTYPE_PCM;
        unknown_subformat[0] = 0x02;
        let wav_bytes = wav_extensible_pcm_i16_bytes(8_000, 1, &[0, 1], unknown_subformat);
        fs::write(&path, wav_bytes).expect("write file");

        let error = decode_audio(&path).expect_err("decode should fail");
        assert!(matches!(error, AppError::UnsupportedFormat(_)));
        assert!(error.to_string().contains("WAV extensible subformat"));

        fs::remove_dir_all(&temp_dir).expect("cleanup temp dir");
    }
}
