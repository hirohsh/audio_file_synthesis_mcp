use crate::error::AppError;

pub fn downmix_to_mono(interleaved_samples: &[f32], channels: u16) -> Result<Vec<f32>, AppError> {
    if channels == 0 {
        return Err(AppError::InvalidParams(
            "channels must be greater than zero".to_string(),
        ));
    }
    let channels = channels as usize;
    if interleaved_samples.is_empty() {
        return Ok(Vec::new());
    }
    if interleaved_samples.len() % channels != 0 {
        return Err(AppError::Format(
            "interleaved sample length is not divisible by channel count".to_string(),
        ));
    }
    if channels == 1 {
        return Ok(interleaved_samples.to_vec());
    }

    let mut mono = Vec::with_capacity(interleaved_samples.len() / channels);
    for frame in interleaved_samples.chunks_exact(channels) {
        let summed: f32 = frame.iter().copied().sum();
        mono.push(summed / channels as f32);
    }
    Ok(mono)
}
