use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

use crate::error::AppError;

pub fn write_wav_mono_i16(path: &Path, sample_rate: u32, samples: &[f32]) -> Result<(), AppError> {
    if sample_rate == 0 {
        return Err(AppError::InvalidParams(
            "sample_rate must be greater than zero".to_string(),
        ));
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|source| AppError::io_with_path(parent, source))?;
        }
    }

    let channels = 1u16;
    let bits_per_sample = 16u16;
    let bytes_per_sample = 2u16;
    let data_size = (samples.len() as u32)
        .checked_mul(bytes_per_sample as u32)
        .ok_or_else(|| AppError::Format("output data size overflow".to_string()))?;
    let fmt_chunk_size = 16u32;
    let riff_size = 4u32
        .checked_add(8 + fmt_chunk_size)
        .and_then(|value| value.checked_add(8 + data_size))
        .ok_or_else(|| AppError::Format("RIFF chunk size overflow".to_string()))?;
    let byte_rate = sample_rate
        .checked_mul(channels as u32)
        .and_then(|value| value.checked_mul(bytes_per_sample as u32))
        .ok_or_else(|| AppError::Format("byte rate overflow".to_string()))?;
    let block_align = channels * bytes_per_sample;

    let file = File::create(path).map_err(|source| AppError::io_with_path(path, source))?;
    let mut writer = BufWriter::new(file);

    writer
        .write_all(b"RIFF")
        .map_err(|source| AppError::io_with_path(path, source))?;
    writer
        .write_all(&riff_size.to_le_bytes())
        .map_err(|source| AppError::io_with_path(path, source))?;
    writer
        .write_all(b"WAVE")
        .map_err(|source| AppError::io_with_path(path, source))?;

    writer
        .write_all(b"fmt ")
        .map_err(|source| AppError::io_with_path(path, source))?;
    writer
        .write_all(&fmt_chunk_size.to_le_bytes())
        .map_err(|source| AppError::io_with_path(path, source))?;
    writer
        .write_all(&1u16.to_le_bytes())
        .map_err(|source| AppError::io_with_path(path, source))?;
    writer
        .write_all(&channels.to_le_bytes())
        .map_err(|source| AppError::io_with_path(path, source))?;
    writer
        .write_all(&sample_rate.to_le_bytes())
        .map_err(|source| AppError::io_with_path(path, source))?;
    writer
        .write_all(&byte_rate.to_le_bytes())
        .map_err(|source| AppError::io_with_path(path, source))?;
    writer
        .write_all(&block_align.to_le_bytes())
        .map_err(|source| AppError::io_with_path(path, source))?;
    writer
        .write_all(&bits_per_sample.to_le_bytes())
        .map_err(|source| AppError::io_with_path(path, source))?;

    writer
        .write_all(b"data")
        .map_err(|source| AppError::io_with_path(path, source))?;
    writer
        .write_all(&data_size.to_le_bytes())
        .map_err(|source| AppError::io_with_path(path, source))?;

    for sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let quantized = if clamped <= -1.0 {
            i16::MIN
        } else if clamped >= 1.0 {
            i16::MAX
        } else {
            (clamped * i16::MAX as f32).round() as i16
        };
        writer
            .write_all(&quantized.to_le_bytes())
            .map_err(|source| AppError::io_with_path(path, source))?;
    }
    writer
        .flush()
        .map_err(|source| AppError::io_with_path(path, source))?;

    Ok(())
}
