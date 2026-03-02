use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::audio::{self, InputAudio, NormalizationOptions, SynthesizeRequest, SynthesizeResult};
use crate::error::AppError;

pub const TOOL_NAME: &str = "synthesize_mono_audio";

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SynthesizeInput {
    pub speaker_id: String,
    pub path: PathBuf,
    #[serde(default)]
    pub gain_db: f32,
    #[serde(default)]
    pub start_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SynthesizeMonoAudioRequest {
    pub inputs: Vec<SynthesizeInput>,
    pub output_path: PathBuf,
    #[serde(default = "default_target_sample_rate")]
    pub target_sample_rate: u32,
    #[serde(default)]
    pub normalization: NormalizationOptions,
    #[serde(default)]
    pub overwrite: bool,
}

fn default_target_sample_rate() -> u32 {
    48_000
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SynthesizeMonoAudioResponse {
    pub output_path: PathBuf,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_ms: u64,
    pub peak_dbfs: f32,
}

pub struct McpServer {
    pub work_dir: PathBuf,
}

impl McpServer {
    pub fn call_synthesize_mono_audio(
        &self,
        request: SynthesizeMonoAudioRequest,
    ) -> Result<SynthesizeMonoAudioResponse, AppError> {
        synthesize_mono_audio(request, &self.work_dir)
    }
}

pub fn synthesize_mono_audio(
    request: SynthesizeMonoAudioRequest,
    work_dir: &std::path::Path,
) -> Result<SynthesizeMonoAudioResponse, AppError> {
    let request = SynthesizeRequest {
        inputs: request
            .inputs
            .into_iter()
            .map(|input| InputAudio {
                speaker_id: input.speaker_id,
                path: input.path,
                gain_db: input.gain_db,
                start_ms: input.start_ms,
            })
            .collect(),
        output_path: request.output_path,
        target_sample_rate: request.target_sample_rate,
        normalization: request.normalization,
        overwrite: request.overwrite,
    };

    let result: SynthesizeResult = audio::synthesize_mono_audio(&request, work_dir)?;
    Ok(SynthesizeMonoAudioResponse {
        output_path: result.output_path,
        sample_rate: result.sample_rate,
        channels: result.channels,
        duration_ms: result.duration_ms,
        peak_dbfs: result.peak_dbfs,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::audio::encode;

    use super::*;

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time moved backwards")
            .as_nanos();
        std::env::temp_dir().join(format!("mcp_tools_test_{}_{}", std::process::id(), nanos))
    }

    #[test]
    fn mcp_tool_call_synthesizes_audio() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");

        let input_a = temp_dir.join("speaker_a.wav");
        let input_b = temp_dir.join("speaker_b.wav");
        let output = temp_dir.join("out.wav");
        encode::write_wav_mono_i16(&input_a, 1_000, &vec![0.2; 120]).expect("write input A");
        encode::write_wav_mono_i16(&input_b, 1_000, &vec![0.1; 50]).expect("write input B");

        let request = SynthesizeMonoAudioRequest {
            inputs: vec![
                SynthesizeInput {
                    speaker_id: "a".to_string(),
                    path: input_a,
                    gain_db: 0.0,
                    start_ms: 0,
                },
                SynthesizeInput {
                    speaker_id: "b".to_string(),
                    path: input_b,
                    gain_db: 0.0,
                    start_ms: 40,
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

        let server = McpServer { work_dir: temp_dir.clone() };
        let response = server
            .call_synthesize_mono_audio(request)
            .expect("tool call must succeed");
        assert_eq!(response.channels, 1);
        assert_eq!(response.sample_rate, 1_000);
        assert!(response.duration_ms >= 120);
        assert!(output.exists());

        fs::remove_dir_all(&temp_dir).expect("cleanup temp dir");
    }
}
