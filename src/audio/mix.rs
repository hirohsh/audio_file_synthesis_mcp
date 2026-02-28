use crate::error::AppError;

#[derive(Copy, Clone, Debug)]
pub struct MixTrack<'a> {
    pub samples: &'a [f32],
    pub start_ms: u64,
    pub gain_db: f32,
}

pub fn mix_tracks(tracks: &[MixTrack<'_>], sample_rate: u32) -> Result<Vec<f32>, AppError> {
    if sample_rate == 0 {
        return Err(AppError::InvalidParams(
            "sample_rate must be greater than zero".to_string(),
        ));
    }
    if tracks.is_empty() {
        return Ok(Vec::new());
    }

    let mut total_len = 0usize;
    for track in tracks {
        let offset = ms_to_samples(track.start_ms, sample_rate);
        total_len = total_len.max(offset + track.samples.len());
    }

    let mut mixed = vec![0.0_f32; total_len];
    for track in tracks {
        let offset = ms_to_samples(track.start_ms, sample_rate);
        let gain = db_to_gain(track.gain_db);
        for (index, sample) in track.samples.iter().enumerate() {
            mixed[offset + index] += *sample * gain;
        }
    }
    Ok(mixed)
}

fn ms_to_samples(ms: u64, sample_rate: u32) -> usize {
    ((ms as u128 * sample_rate as u128) / 1_000_u128) as usize
}

fn db_to_gain(db: f32) -> f32 {
    10_f32.powf(db / 20.0)
}
