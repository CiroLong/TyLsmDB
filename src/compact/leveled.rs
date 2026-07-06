use crate::options::Options;

pub fn max_bytes_for_level(options: &Options, level: usize) -> u64 {
    if level <= 1 {
        return options.max_bytes_for_level_base as u64;
    }

    let multiplier = options
        .max_bytes_for_level_multiplier
        .powi((level - 1) as i32);
    ((options.max_bytes_for_level_base as f64) * multiplier) as u64
}
