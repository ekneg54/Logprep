/// Range boundary types and parsing for filter expressions.
///
/// Supports integer, float, and string ranges with inclusive/exclusive boundaries.

/// Parse a string boundary as i64, returning None if not a valid integer.
pub fn parse_int_range(lower: &str, upper: &str) -> Option<(i64, i64)> {
    let lo = lower.parse::<i64>().ok()?;
    let hi = upper.parse::<i64>().ok()?;
    Some((lo, hi))
}

/// Parse string boundaries as f64, returning None if not valid finite floats.
pub fn parse_float_range(lower: &str, upper: &str) -> Option<(f64, f64)> {
    let lo = lower.parse::<f64>().ok()?;
    let hi = upper.parse::<f64>().ok()?;
    if !lo.is_finite() || !hi.is_finite() {
        return None;
    }
    Some((lo, hi))
}

/// Check if a boundary value looks like a finite numeric string.
pub fn is_finite_numeric_boundary(value: &str) -> bool {
    match value.parse::<f64>() {
        Ok(v) => v.is_finite(),
        Err(_) => false,
    }
}

/// Parse a string range, returning None if either boundary is "*" or if
/// one boundary is numeric (to avoid mixed-type ranges).
pub fn parse_string_range(lower: &str, upper: &str) -> Option<(String, String)> {
    if lower == "*" || upper == "*" {
        return None;
    }
    if is_finite_numeric_boundary(lower) || is_finite_numeric_boundary(upper) {
        return None;
    }
    Some((lower.to_string(), upper.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_int_range_valid() {
        assert_eq!(parse_int_range("0", "10"), Some((0, 10)));
        assert_eq!(parse_int_range("-10", "-1"), Some((-10, -1)));
    }

    #[test]
    fn test_parse_int_range_invalid() {
        assert_eq!(parse_int_range("abc", "10"), None);
        assert_eq!(parse_int_range("0", "xyz"), None);
    }

    #[test]
    fn test_parse_float_range_valid() {
        assert_eq!(parse_float_range("0.1", "8.5"), Some((0.1, 8.5)));
        assert_eq!(parse_float_range("-1.5", "1.5"), Some((-1.5, 1.5)));
    }

    #[test]
    fn test_parse_float_range_invalid() {
        assert_eq!(parse_float_range("abc", "1.0"), None);
        assert_eq!(parse_float_range("0", "nan"), None);
        assert_eq!(parse_float_range("-inf", "10"), None);
    }

    #[test]
    fn test_is_finite_numeric_boundary() {
        assert!(is_finite_numeric_boundary("42"));
        assert!(is_finite_numeric_boundary("3.14"));
        assert!(!is_finite_numeric_boundary("nan"));
        assert!(!is_finite_numeric_boundary("inf"));
        assert!(!is_finite_numeric_boundary("abc"));
    }

    #[test]
    fn test_parse_string_range_valid() {
        assert_eq!(
            parse_string_range("alpha", "zulu"),
            Some(("alpha".to_string(), "zulu".to_string()))
        );
    }

    #[test]
    fn test_parse_string_range_with_star() {
        assert_eq!(parse_string_range("*", "zulu"), None);
        assert_eq!(parse_string_range("alpha", "*"), None);
    }

    #[test]
    fn test_parse_string_range_with_numeric() {
        assert_eq!(parse_string_range("42", "zulu"), None);
        assert_eq!(parse_string_range("alpha", "10"), None);
    }
}
