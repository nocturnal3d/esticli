use std::sync::LazyLock;

use human_format::{Formatter, Scales};

// Built once and reused. Constructing a `Formatter` allocates its whole suffix
// table (two dozen small strings), which made building one per call ~5x the
// cost of the formatting itself — and these run several times per on-screen
// row on every frame.
static NUMBER_FORMATTER: LazyLock<Formatter> = LazyLock::new(|| {
    let mut formatter = Formatter::new();
    formatter.with_decimals(1).with_separator("");
    formatter
});

static BYTES_FORMATTER: LazyLock<Formatter> = LazyLock::new(|| {
    let mut formatter = Formatter::new();
    formatter
        .with_decimals(1)
        .with_separator(" ")
        .with_scales(Scales::Binary())
        .with_units("B");
    formatter
});

// Format a number with SI suffixes (K, M, B, T)
pub fn format_number(value: f64) -> String {
    // Below 1, `human_format` reaches for the *sub*-unit SI prefixes and
    // renders 0.8251 as "825.1m" — milli-units, which reads as "minutes" in a
    // column headed "Rate (/s)". Nothing here is ever usefully expressed in
    // thousandths, so small values are just printed as plain decimals.
    if value.abs() < 1.0 {
        return format!("{:.1}", value);
    }

    NUMBER_FORMATTER.format(value)
}

// Format bytes with binary suffixes (KB, MB, GB, TB)
pub fn format_bytes(bytes: u64) -> String {
    BYTES_FORMATTER.format(bytes as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_number_avoids_sub_unit_prefixes() {
        // A sub-1 rate must not come back as milli-units.
        assert_eq!(format_number(0.8251), "0.8");
        assert_eq!(format_number(0.4), "0.4");
        assert_eq!(format_number(0.0), "0.0");
        // Values at or above 1 keep the usual SI scaling.
        assert_eq!(format_number(1.0), "1.0");
        assert_eq!(format_number(999.0), "999.0");
        assert_eq!(format_number(1500.0), "1.5k");
    }

    #[test]
    fn test_format_bytes_uses_binary_units() {
        assert_eq!(format_bytes(0), "0.0 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(5 * 1024 * 1024 * 1024), "5.0 GiB");
    }
}
