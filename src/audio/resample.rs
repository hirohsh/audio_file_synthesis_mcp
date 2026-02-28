use crate::error::AppError;

pub fn resample_linear(
    input: &[f32],
    source_rate: u32,
    target_rate: u32,
) -> Result<Vec<f32>, AppError> {
    if source_rate == 0 || target_rate == 0 {
        return Err(AppError::InvalidParams(
            "source_rate and target_rate must be greater than zero".to_string(),
        ));
    }
    if input.is_empty() {
        return Ok(Vec::new());
    }
    if source_rate == target_rate {
        return Ok(input.to_vec());
    }

    let output_len = ((input.len() as u128 * target_rate as u128 + source_rate as u128 - 1)
        / source_rate as u128) as usize;
    if output_len == 0 {
        return Ok(Vec::new());
    }

    let mut output = Vec::with_capacity(output_len);
    let last_index = input.len() - 1;
    for i in 0..output_len {
        let src_pos = i as f64 * source_rate as f64 / target_rate as f64;
        let index = src_pos.floor() as usize;
        if index >= last_index {
            output.push(input[last_index]);
            continue;
        }

        let frac = (src_pos - index as f64) as f32;
        let left = input[index];
        let right = input[index + 1];
        output.push(left + (right - left) * frac);
    }
    Ok(output)
}
