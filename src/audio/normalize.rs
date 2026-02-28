use crate::error::AppError;

pub fn apply_peak_normalization(
    samples: &mut [f32],
    target_peak_dbfs: f32,
) -> Result<(), AppError> {
    if !target_peak_dbfs.is_finite() || target_peak_dbfs > 0.0 {
        return Err(AppError::InvalidParams(
            "target_peak_dbfs must be finite and <= 0.0".to_string(),
        ));
    }

    let current_peak = peak_amplitude(samples);
    if current_peak == 0.0 {
        return Ok(());
    }

    let target_peak = 10_f32.powf(target_peak_dbfs / 20.0);
    let scale = target_peak / current_peak;
    for sample in samples {
        *sample *= scale;
    }
    Ok(())
}

pub fn peak_dbfs(samples: &[f32]) -> f32 {
    let peak = peak_amplitude(samples);
    if peak == 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * peak.log10()
    }
}

fn peak_amplitude(samples: &[f32]) -> f32 {
    samples
        .iter()
        .copied()
        .map(f32::abs)
        .fold(0.0_f32, f32::max)
}
