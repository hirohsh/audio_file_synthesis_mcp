pub mod decode;
pub mod downmix;
pub mod encode;
pub mod mix;
pub mod normalize;
pub mod resample;

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use mix::MixTrack;

const MAX_SAMPLE_RATE: u32 = 192_000;
const MAX_START_MS: u64 = 3_600_000;
/// Maximum combined size of all input files (200 MB).
const MAX_TOTAL_FILE_SIZE: u64 = 200 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct InputAudio {
    pub speaker_id: String,
    pub path: PathBuf,
    pub gain_db: f32,
    pub start_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct NormalizationOptions {
    #[serde(default = "default_normalization_enabled")]
    pub enabled: bool,
    #[serde(default = "default_peak_dbfs")]
    pub peak_dbfs: f32,
}

/// Resolve `path` relative to `base` (if relative) and normalize away `.` / `..` components
/// without calling `canonicalize` (which requires the path to already exist).
fn normalize_path(base: &Path, path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };

    let mut result = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {}
            c => result.push(c),
        }
    }
    result
}

/// Return `Err` if `path` (resolved relative to `base`) falls outside `base`.
fn validate_path_within_work_dir(
    path: &Path,
    work_dir: &Path,
    label: &str,
) -> Result<PathBuf, AppError> {
    let resolved = normalize_path(work_dir, path);
    if !resolved.starts_with(work_dir) {
        return Err(AppError::InvalidParams(format!(
            "{label} must be located inside the working directory: {}",
            path.display()
        )));
    }
    Ok(resolved)
}

fn default_normalization_enabled() -> bool {
    true
}

fn default_peak_dbfs() -> f32 {
    -1.0
}

impl Default for NormalizationOptions {
    fn default() -> Self {
        Self {
            enabled: default_normalization_enabled(),
            peak_dbfs: default_peak_dbfs(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SynthesizeRequest {
    pub inputs: Vec<InputAudio>,
    pub output_path: PathBuf,
    pub target_sample_rate: u32,
    pub normalization: NormalizationOptions,
    pub overwrite: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SynthesizeResult {
    pub output_path: PathBuf,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_ms: u64,
    pub peak_dbfs: f32,
}

pub fn synthesize_mono_audio(
    request: &SynthesizeRequest,
    work_dir: &Path,
) -> Result<SynthesizeResult, AppError> {
    if request.inputs.is_empty() {
        return Err(AppError::InvalidParams(
            "`inputs` must contain at least one audio file".to_string(),
        ));
    }

    if request.target_sample_rate == 0 || request.target_sample_rate > MAX_SAMPLE_RATE {
        return Err(AppError::InvalidParams(format!(
            "`target_sample_rate` must be between 1 and {}",
            MAX_SAMPLE_RATE
        )));
    }

    let resolved_output =
        validate_path_within_work_dir(&request.output_path, work_dir, "`output_path`")?;

    if !request.overwrite && resolved_output.exists() {
        return Err(AppError::InvalidParams(format!(
            "`output_path` already exists and overwrite is disabled: {}",
            resolved_output.display()
        )));
    }

    if request.normalization.enabled
        && (!request.normalization.peak_dbfs.is_finite() || request.normalization.peak_dbfs > 0.0)
    {
        return Err(AppError::InvalidParams(
            "`normalization.peak_dbfs` must be a finite value <= 0.0".to_string(),
        ));
    }

    // Validate all input paths and accumulate total file size before decoding.
    let mut total_input_size: u64 = 0;
    for input in &request.inputs {
        if input.start_ms > MAX_START_MS {
            return Err(AppError::InvalidParams(format!(
                "start_ms for {} exceeds maximum of {} ms",
                input.path.display(),
                MAX_START_MS
            )));
        }

        validate_path_within_work_dir(&input.path, work_dir, "input path")?;

        if !input.path.exists() {
            return Err(AppError::InvalidParams(format!(
                "input file does not exist: {}",
                input.path.display()
            )));
        }

        let file_size = std::fs::metadata(&input.path)
            .map_err(|source| AppError::io_with_path(&input.path, source))?
            .len();
        total_input_size = total_input_size.saturating_add(file_size);
        if total_input_size > MAX_TOTAL_FILE_SIZE {
            return Err(AppError::InvalidParams(format!(
                "combined input file size exceeds the maximum of {} bytes",
                MAX_TOTAL_FILE_SIZE
            )));
        }
    }

    let mut prepared_tracks: Vec<(Vec<f32>, u64, f32)> = Vec::with_capacity(request.inputs.len());
    for input in &request.inputs {
        let decoded = decode::decode_audio(&input.path)?;
        let mono = downmix::downmix_to_mono(&decoded.samples, decoded.channels)?;
        let resampled =
            resample::resample_linear(&mono, decoded.sample_rate, request.target_sample_rate)?;
        prepared_tracks.push((resampled, input.start_ms, input.gain_db));
    }

    let mix_tracks: Vec<MixTrack<'_>> = prepared_tracks
        .iter()
        .map(|(samples, start_ms, gain_db)| MixTrack {
            samples,
            start_ms: *start_ms,
            gain_db: *gain_db,
        })
        .collect();
    let mut mixed = mix::mix_tracks(&mix_tracks, request.target_sample_rate)?;

    if request.normalization.enabled {
        normalize::apply_peak_normalization(&mut mixed, request.normalization.peak_dbfs)?;
    }

    let peak_dbfs = normalize::peak_dbfs(&mixed);
    encode::write_wav_mono_i16(&resolved_output, request.target_sample_rate, &mixed)?;

    let duration_ms =
        ((mixed.len() as u128 * 1_000_u128) / request.target_sample_rate as u128) as u64;

    Ok(SynthesizeResult {
        output_path: resolved_output,
        sample_rate: request.target_sample_rate,
        channels: 1,
        duration_ms,
        peak_dbfs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn approx_eq(actual: f32, expected: f32, epsilon: f32) {
        assert!(
            (actual - expected).abs() <= epsilon,
            "expected {expected}, got {actual}"
        );
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time moved backwards")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "audio_file_synthesis_mcp_test_{}_{}",
            std::process::id(),
            nanos
        ))
    }

    #[test]
    fn downmix_stereo_to_mono() {
        let stereo = vec![1.0, -1.0, 0.5, 0.5];
        let mono = downmix::downmix_to_mono(&stereo, 2).expect("downmix must succeed");
        assert_eq!(mono.len(), 2);
        approx_eq(mono[0], 0.0, 1e-6);
        approx_eq(mono[1], 0.5, 1e-6);
    }

    #[test]
    fn linear_resample_upsamples() {
        let input = vec![0.0, 1.0];
        let output = resample::resample_linear(&input, 2, 4).expect("resample must succeed");
        assert_eq!(output.len(), 4);
        approx_eq(output[0], 0.0, 1e-6);
        approx_eq(output[1], 0.5, 1e-6);
        approx_eq(output[2], 1.0, 1e-6);
        approx_eq(output[3], 1.0, 1e-6);
    }

    #[test]
    fn mix_applies_offsets() {
        let track_a = vec![1.0, 1.0];
        let track_b = vec![1.0];
        let tracks = vec![
            MixTrack {
                samples: &track_a,
                start_ms: 0,
                gain_db: 0.0,
            },
            MixTrack {
                samples: &track_b,
                start_ms: 500,
                gain_db: 0.0,
            },
        ];

        let mixed = mix::mix_tracks(&tracks, 2).expect("mix must succeed");
        assert_eq!(mixed, vec![1.0, 2.0]);
    }

    #[test]
    fn peak_normalization_targets_requested_peak() {
        let mut samples = vec![0.2, -0.4, 0.1];
        normalize::apply_peak_normalization(&mut samples, -6.0206).expect("normalize must work");
        let peak = samples
            .iter()
            .copied()
            .map(f32::abs)
            .fold(0.0_f32, f32::max);
        approx_eq(peak, 0.5, 5e-3);
    }

    #[test]
    fn end_to_end_synthesizes_wav_output() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let input_a = temp_dir.join("a.wav");
        let input_b = temp_dir.join("b.wav");
        let output = temp_dir.join("mixed.wav");

        let speaker_a = vec![0.2; 200];
        let speaker_b = vec![0.2; 100];
        encode::write_wav_mono_i16(&input_a, 1_000, &speaker_a).expect("write wav A");
        encode::write_wav_mono_i16(&input_b, 1_000, &speaker_b).expect("write wav B");

        let request = SynthesizeRequest {
            inputs: vec![
                InputAudio {
                    speaker_id: "spk-a".to_string(),
                    path: input_a.clone(),
                    gain_db: 0.0,
                    start_ms: 0,
                },
                InputAudio {
                    speaker_id: "spk-b".to_string(),
                    path: input_b.clone(),
                    gain_db: 0.0,
                    start_ms: 100,
                },
            ],
            output_path: output.clone(),
            target_sample_rate: 1_000,
            normalization: NormalizationOptions {
                enabled: false,
                peak_dbfs: -1.0,
            },
            overwrite: false,
        };

        let result = synthesize_mono_audio(&request, &temp_dir).expect("synthesis must succeed");
        assert_eq!(result.channels, 1);
        assert_eq!(result.sample_rate, 1_000);
        assert_eq!(result.duration_ms, 200);
        assert!(result.peak_dbfs.is_finite());
        assert!(output.exists());

        let decoded = decode::decode_audio(&output).expect("decode output wav");
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.sample_rate, 1_000);
        assert_eq!(decoded.samples.len(), 200);

        fs::remove_dir_all(&temp_dir).expect("cleanup temp dir");
    }
}

