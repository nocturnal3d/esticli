use human_format::{Formatter, Scales};

// Format a number with SI suffixes (K, M, B, T)
pub fn format_number(value: f64) -> String {
    Formatter::new()
        .with_decimals(1)
        .with_separator("")
        .format(value)
}

// Format bytes with binary suffixes (KB, MB, GB, TB)
pub fn format_bytes(bytes: u64) -> String {
    Formatter::new()
        .with_decimals(1)
        .with_separator(" ")
        .with_scales(Scales::Binary())
        .with_units("B")
        .format(bytes as f64)
}
