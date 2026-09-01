// small standalone helpers that don't belong to any one module.

// formats a count the way web UIs do: "265K", "1.5M", "2.1B"; plain
// number under 1000. one decimal, dropped when it'd be ".0" (1.0M -> 1M).
// used for Modrinth/CurseForge download counts in the browse popups, which
// otherwise render as a raw "18453212 downloads" that blows out the row.
#[must_use]
pub fn format_count(n: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1_000_000_000, "B"), (1_000_000, "M"), (1_000, "K")];

    for (threshold, suffix) in UNITS {
        if n >= threshold {
            let scaled = n as f64 / threshold as f64;
            // round first so 999_950 doesn't print as "1000.0K" — re-check
            // the threshold after rounding and fall through to the next
            // (larger) unit if it rolled over.
            let rounded = (scaled * 10.0).round() / 10.0;
            if rounded >= 1000.0 {
                continue;
            }
            return if (rounded - rounded.trunc()).abs() < f64::EPSILON {
                format!("{}{}", rounded as u64, suffix)
            } else {
                format!("{rounded:.1}{suffix}")
            };
        }
    }
    n.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_1000_is_plain() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
    }

    #[test]
    fn thousands() {
        assert_eq!(format_count(1_000), "1K");
        assert_eq!(format_count(1_500), "1.5K");
        assert_eq!(format_count(265_000), "265K");
        assert_eq!(format_count(265_432), "265.4K");
    }

    #[test]
    fn millions() {
        assert_eq!(format_count(1_000_000), "1M");
        assert_eq!(format_count(1_500_000), "1.5M");
        assert_eq!(format_count(18_453_212), "18.5M");
    }

    #[test]
    fn billions() {
        assert_eq!(format_count(2_100_000_000), "2.1B");
    }

    #[test]
    fn rounds_up_across_threshold() {
        // 999_950 rounds to 1000.0K, which should bump to the M unit.
        assert_eq!(format_count(999_950), "1M");
    }
}
