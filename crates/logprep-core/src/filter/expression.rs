use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use fancy_regex::Regex as FilterRegex;
use regex::Regex as StdRegex;
use serde_json::{Map, Value};

// ─── Exceptions (Python-side defined, Rust uses PyErr) ───

/// Helper to raise KeyDoesNotExistError from Rust.
fn raise_key_does_not_exist_error(py: Python, msg: &str) -> pyo3::PyErr {
    let result = (|| -> PyResult<pyo3::PyErr> {
        let module = py.import("logprep.filter.expression")?;
        let error_class = module.getattr("KeyDoesNotExistError")?;
        let instance = error_class.call1((msg,))?;
        Ok(pyo3::PyErr::from_value(instance))
    })();
    result.unwrap_or_else(|e| e)
}

fn format_float(value: f64) -> String {
    let mut s = value.to_string();
    if !s.contains('.') && !s.contains('e') && !s.contains('E') {
        s.push_str(".0");
    }
    s
}

// ─── Numerische Range-Grenzen (beliebige Präzision) ───

/// Eine numerische Range-Grenze: Ganzzahl (als kanonischer Dezimalstring für
/// beliebige Präzision) oder Fließkommazahl.
#[derive(Debug, Clone)]
pub enum NumericBound {
    Int(String),
    Float(f64),
}

impl NumericBound {
    /// Parse eine Grenze aus einem String: erst als Ganzzahl (beliebige
    /// Präzision), dann als endliche Fließkommazahl. Sonst None.
    pub fn parse(s: &str) -> Option<NumericBound> {
        if let Some(canonical) = canonicalize_int_str(s) {
            return Some(NumericBound::Int(canonical));
        }
        if let Ok(f) = s.parse::<f64>() {
            if f.is_finite() {
                return Some(NumericBound::Float(f));
            }
        }
        None
    }

    fn as_f64(&self) -> f64 {
        match self {
            NumericBound::Int(s) => s.parse::<f64>().unwrap_or(f64::NAN),
            NumericBound::Float(f) => *f,
        }
    }

    /// Numerischer Vergleich: Int-Int exakt (Stringvergleich beliebiger
    /// Präzision), sonst über f64.
    pub fn compare(&self, other: &NumericBound) -> std::cmp::Ordering {
        match (self, other) {
            (NumericBound::Int(a), NumericBound::Int(b)) => compare_int_strings(a, b),
            _ => self
                .as_f64()
                .partial_cmp(&other.as_f64())
                .unwrap_or(std::cmp::Ordering::Equal),
        }
    }

    pub fn to_repr(&self) -> String {
        match self {
            NumericBound::Int(s) => s.clone(),
            NumericBound::Float(f) => format_float(*f),
        }
    }
}

/// Kanonisiert einen Ganzzahl-String (optionales '-', Ziffern, keine
/// führenden Nullen, "-0" → "0"). Gibt None zurück, wenn kein Ganzzahl-String.
fn canonicalize_int_str(s: &str) -> Option<String> {
    let (negative, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let stripped = digits.trim_start_matches('0');
    if stripped.is_empty() {
        return Some("0".to_string());
    }
    if negative {
        Some(format!("-{}", stripped))
    } else {
        Some(stripped.to_string())
    }
}

/// Vergleicht zwei kanonisierte Ganzzahl-Strings exakt (beliebige Präzision).
fn compare_int_strings(a: &str, b: &str) -> std::cmp::Ordering {
    let (a_neg, a_digits) = (a.starts_with('-'), a.trim_start_matches('-'));
    let (b_neg, b_digits) = (b.starts_with('-'), b.trim_start_matches('-'));
    match (a_neg, b_neg) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => a_digits
            .len()
            .cmp(&b_digits.len())
            .then_with(|| a_digits.cmp(b_digits)),
        (true, true) => b_digits
            .len()
            .cmp(&a_digits.len())
            .then_with(|| b_digits.cmp(a_digits)),
    }
}

// ═══════════════════════════════════════════════════════════
// Pure Rust Core — Kein PyO3, nur serde_json
// ═══════════════════════════════════════════════════════════

/// Alle 15 Expression-Varianten als Rust-Enum.
/// Match-Logik arbeitet auf `serde_json::Value` (keine Python-Objekte).
#[derive(Debug, Clone)]
pub enum FilterExpressionInner {
    Always {
        value: bool,
    },
    Not {
        child: Box<FilterExpressionInner>,
    },
    And {
        children: Vec<FilterExpressionInner>,
    },
    Or {
        children: Vec<FilterExpressionInner>,
    },
    String {
        key: Vec<String>,
        expected: String,
    },
    Wildcard {
        key: Vec<String>,
        expected: String,
        regex: FilterRegex,
    },
    Sigma {
        key: Vec<String>,
        expected: String,
        regex: FilterRegex,
    },
    Integer {
        key: Vec<String>,
        expected: i64,
    },
    Float {
        key: Vec<String>,
        expected: f64,
    },
    IntegerRange {
        key: Vec<String>,
        lower: i64,
        upper: i64,
        incl_low: bool,
        incl_high: bool,
    },
    FloatRange {
        key: Vec<String>,
        lower: f64,
        upper: f64,
        incl_low: bool,
        incl_high: bool,
    },
    NumericRange {
        key: Vec<String>,
        lower: Option<NumericBound>,
        upper: Option<NumericBound>,
        incl_low: bool,
        incl_high: bool,
    },
    StringRange {
        key: Vec<String>,
        lower: Option<String>,
        upper: Option<String>,
        incl_low: bool,
        incl_high: bool,
    },
    Regex {
        key: Vec<String>,
        pattern: FilterRegex,
    },
    Exists {
        key: Vec<String>,
    },
    Null {
        key: Vec<String>,
    },
}

impl PartialEq for FilterExpressionInner {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Always { value: a }, Self::Always { value: b }) => a == b,
            (Self::Not { child: a }, Self::Not { child: b }) => a == b,
            (Self::And { children: a }, Self::And { children: b }) => a == b,
            (Self::Or { children: a }, Self::Or { children: b }) => a == b,
            (
                Self::String {
                    key: a_k,
                    expected: a_v,
                },
                Self::String {
                    key: b_k,
                    expected: b_v,
                },
            ) => a_k == b_k && a_v == b_v,
            (
                Self::Wildcard {
                    key: a_k,
                    expected: a_v,
                    ..
                },
                Self::Wildcard {
                    key: b_k,
                    expected: b_v,
                    ..
                },
            ) => a_k == b_k && a_v == b_v,
            (
                Self::Sigma {
                    key: a_k,
                    expected: a_v,
                    ..
                },
                Self::Sigma {
                    key: b_k,
                    expected: b_v,
                    ..
                },
            ) => a_k == b_k && a_v == b_v,
            (
                Self::Integer {
                    key: a_k,
                    expected: a_v,
                },
                Self::Integer {
                    key: b_k,
                    expected: b_v,
                },
            ) => a_k == b_k && a_v == b_v,
            (
                Self::Float {
                    key: a_k,
                    expected: a_v,
                },
                Self::Float {
                    key: b_k,
                    expected: b_v,
                },
            ) => a_k == b_k && a_v == b_v,
            (
                Self::IntegerRange {
                    key: a_k,
                    lower: a_l,
                    upper: a_u,
                    incl_low: a_il,
                    incl_high: a_ih,
                },
                Self::IntegerRange {
                    key: b_k,
                    lower: b_l,
                    upper: b_u,
                    incl_low: b_il,
                    incl_high: b_ih,
                },
            ) => a_k == b_k && a_l == b_l && a_u == b_u && a_il == b_il && a_ih == b_ih,
            (
                Self::FloatRange {
                    key: a_k,
                    lower: a_l,
                    upper: a_u,
                    incl_low: a_il,
                    incl_high: a_ih,
                },
                Self::FloatRange {
                    key: b_k,
                    lower: b_l,
                    upper: b_u,
                    incl_low: b_il,
                    incl_high: b_ih,
                },
            ) => a_k == b_k && a_l == b_l && a_u == b_u && a_il == b_il && a_ih == b_ih,
            (
                Self::NumericRange {
                    key: a_k,
                    lower: a_l,
                    upper: a_u,
                    incl_low: a_il,
                    incl_high: a_ih,
                },
                Self::NumericRange {
                    key: b_k,
                    lower: b_l,
                    upper: b_u,
                    incl_low: b_il,
                    incl_high: b_ih,
                },
            ) => {
                a_k == b_k
                    && numeric_bound_eq(a_l, b_l)
                    && numeric_bound_eq(a_u, b_u)
                    && a_il == b_il
                    && a_ih == b_ih
            }
            (
                Self::StringRange {
                    key: a_k,
                    lower: a_l,
                    upper: a_u,
                    incl_low: a_il,
                    incl_high: a_ih,
                },
                Self::StringRange {
                    key: b_k,
                    lower: b_l,
                    upper: b_u,
                    incl_low: b_il,
                    incl_high: b_ih,
                },
            ) => a_k == b_k && a_l == b_l && a_u == b_u && a_il == b_il && a_ih == b_ih,
            (Self::Regex { key: a_k, .. }, Self::Regex { key: b_k, .. }) => a_k == b_k,
            (Self::Exists { key: a }, Self::Exists { key: b }) => a == b,
            (Self::Null { key: a }, Self::Null { key: b }) => a == b,
            _ => false,
        }
    }
}

/// Vergleicht zwei optionale numerische Grenzen auf Gleichheit.
fn numeric_bound_eq(a: &Option<NumericBound>, b: &Option<NumericBound>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => match (x, y) {
            (NumericBound::Int(s1), NumericBound::Int(s2)) => s1 == s2,
            (NumericBound::Float(f1), NumericBound::Float(f2)) => f1 == f2,
            _ => false,
        },
        _ => false,
    }
}

/// Fehler-Typ (kein Python, pure Rust)
#[derive(Debug)]
pub enum MatchError {
    KeyNotFound,
    TypeMismatch,
}

impl FilterExpressionInner {
    /// Safe-Matching: Gibt False bei fehlenden Keys/Typfehlern zurück.
    pub fn matches(&self, document: &Value) -> bool {
        match self.does_match(document) {
            Ok(result) => result,
            Err(MatchError::KeyNotFound) => false,
            Err(MatchError::TypeMismatch) => false,
        }
    }

    /// Fallibles Matching — wirft MatchError bei fehlenden Keys.
    pub fn does_match(&self, document: &Value) -> Result<bool, MatchError> {
        match self {
            Self::Always { value } => Ok(*value),

            Self::Not { child } => Ok(!child.matches(document)),

            Self::And { children } => {
                for child in children {
                    if !child.matches(document) {
                        return Ok(false);
                    }
                }
                Ok(true)
            }

            Self::Or { children } => {
                for child in children {
                    if child.matches(document) {
                        return Ok(true);
                    }
                }
                Ok(false)
            }

            Self::String { key, expected } => {
                let value = get_json_value(key, document)?;
                match value {
                    Value::String(s) => Ok(s == expected),
                    Value::Array(arr) => {
                        Ok(arr.iter().any(|v| v.as_str() == Some(expected.as_str())))
                    }
                    Value::Number(n) => Ok(&n.to_string() == expected),
                    Value::Bool(b) => Ok(&b.to_string() == expected),
                    _ => Ok(false),
                }
            }

            Self::Wildcard { key, regex, .. } | Self::Sigma { key, regex, .. } => {
                let value = get_json_value(key, document)?;
                match value {
                    Value::String(s) => Ok(regex.is_match(s).unwrap_or(false)),
                    Value::Array(arr) => Ok(arr
                        .iter()
                        .any(|v| v.as_str().map_or(false, |s| regex.is_match(s).unwrap_or(false)))),
                    _ => Ok(false),
                }
            }

            Self::Integer { key, expected } => {
                let value = get_json_value(key, document)?;
                match value {
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            Ok(i == *expected)
                        } else if let Some(f) = n.as_f64() {
                            Ok(f as i64 == *expected)
                        } else {
                            Ok(false)
                        }
                    }
                    _ => Ok(false),
                }
            }

            Self::Float { key, expected } => {
                let value = get_json_value(key, document)?;
                match value {
                    Value::Number(n) => {
                        if let Some(f) = n.as_f64() {
                            Ok((f - *expected).abs() < f64::EPSILON)
                        } else {
                            Ok(false)
                        }
                    }
                    _ => Ok(false),
                }
            }

            Self::IntegerRange {
                key,
                lower,
                upper,
                incl_low,
                incl_high,
            } => {
                let value = get_json_value(key, document)?;
                match value {
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            let lo_ok = if *incl_low { i >= *lower } else { i > *lower };
                            let hi_ok = if *incl_high { i <= *upper } else { i < *upper };
                            Ok(lo_ok && hi_ok)
                        } else {
                            Ok(false)
                        }
                    }
                    _ => Ok(false),
                }
            }

            Self::FloatRange {
                key,
                lower,
                upper,
                incl_low,
                incl_high,
            } => {
                let value = get_json_value(key, document)?;
                match value {
                    Value::Number(n) => {
                        if let Some(f) = n.as_f64() {
                            let lo_ok = if *incl_low { f >= *lower } else { f > *lower };
                            let hi_ok = if *incl_high { f <= *upper } else { f < *upper };
                            Ok(lo_ok && hi_ok)
                        } else {
                            Ok(false)
                        }
                    }
                    _ => Ok(false),
                }
            }

            Self::NumericRange {
                key,
                lower,
                upper,
                incl_low,
                incl_high,
            } => {
                let value = get_json_value(key, document)?;
                let numeric = match value {
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            Some(NumericBound::Int(i.to_string()))
                        } else {
                            n.as_f64().map(NumericBound::Float)
                        }
                    }
                    Value::String(s) => NumericBound::parse(s),
                    _ => None,
                };
                match numeric {
                    None => Ok(false),
                    Some(v) => {
                        let lo_ok = match lower {
                            None => true,
                            Some(lo) => {
                                let ord = v.compare(lo);
                                if *incl_low {
                                    ord != std::cmp::Ordering::Less
                                } else {
                                    ord == std::cmp::Ordering::Greater
                                }
                            }
                        };
                        let hi_ok = match upper {
                            None => true,
                            Some(hi) => {
                                let ord = v.compare(hi);
                                if *incl_high {
                                    ord != std::cmp::Ordering::Greater
                                } else {
                                    ord == std::cmp::Ordering::Less
                                }
                            }
                        };
                        Ok(lo_ok && hi_ok)
                    }
                }
            }

            Self::StringRange {
                key,
                lower,
                upper,
                incl_low,
                incl_high,
            } => {
                let value = get_json_value(key, document)?;
                let string_value = match value {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            i.to_string()
                        } else if let Some(f) = n.as_f64() {
                            format_float(f)
                        } else {
                            return Ok(false);
                        }
                    }
                    _ => return Ok(false),
                };
                let s = string_value.as_str();
                let lo_ok = match lower {
                    None => true,
                    Some(lo) => {
                        if *incl_low {
                            s >= lo.as_str()
                        } else {
                            s > lo.as_str()
                        }
                    }
                };
                let hi_ok = match upper {
                    None => true,
                    Some(hi) => {
                        if *incl_high {
                            s <= hi.as_str()
                        } else {
                            s < hi.as_str()
                        }
                    }
                };
                Ok(lo_ok && hi_ok)
            }

            Self::Regex { key, pattern } => {
                let value = get_json_value(key, document)?;
                match value {
                    Value::String(s) => Ok(pattern.is_match(s).unwrap_or(false)),
                    Value::Array(arr) => Ok(arr
                        .iter()
                        .any(|v| v.as_str().map_or(false, |s| pattern.is_match(s).unwrap_or(false)))),
                    _ => Ok(false),
                }
            }

            Self::Exists { key } => Ok(path_exists(key, document)),

            Self::Null { key } => {
                let value = get_json_value(key, document)?;
                Ok(value.is_null())
            }
        }
    }

    /// Python-kompatibles __repr__ (pure Rust, kein Python-Aufruf).
    pub fn to_repr(&self) -> String {
        match self {
            Self::Always { value } => {
                if *value {
                    "*".to_string()
                } else {
                    "".to_string()
                }
            }
            Self::Not { child } => format!("NOT ({})", child.to_repr()),
            Self::And { children } => {
                let parts: Vec<String> = children.iter().map(|c| c.to_repr()).collect();
                format!("({})", parts.join(" AND "))
            }
            Self::Or { children } => {
                let parts: Vec<String> = children.iter().map(|c| c.to_repr()).collect();
                format!("({})", parts.join(" OR "))
            }
            Self::String { key, expected } => {
                format!("{}:\"{}\"", dotted_key(key), expected)
            }
            Self::Wildcard { key, expected, .. } => {
                format!("{}:\"{}\"", dotted_key(key), expected)
            }
            Self::Sigma { key, expected, .. } => {
                format!("{}:\"{}\"", dotted_key(key), expected)
            }
            Self::Integer { key, expected } => {
                format!("{}:{}", dotted_key(key), expected)
            }
            Self::Float { key, expected } => {
                format!("{}:{}", dotted_key(key), format_float(*expected))
            }
            Self::IntegerRange {
                key,
                lower,
                upper,
                incl_low,
                incl_high,
            } => {
                range_repr(
                    key,
                    &lower.to_string(),
                    &upper.to_string(),
                    *incl_low,
                    *incl_high,
                )
            }
            Self::FloatRange {
                key,
                lower,
                upper,
                incl_low,
                incl_high,
            } => {
                range_repr(
                    key,
                    &format_float(*lower),
                    &format_float(*upper),
                    *incl_low,
                    *incl_high,
                )
            }
            Self::NumericRange {
                key,
                lower,
                upper,
                incl_low,
                incl_high,
            } => {
                let lo = lower
                    .as_ref()
                    .map_or("*".to_string(), |b| b.to_repr());
                let hi = upper
                    .as_ref()
                    .map_or("*".to_string(), |b| b.to_repr());
                range_repr(key, &lo, &hi, *incl_low, *incl_high)
            }
            Self::StringRange {
                key,
                lower,
                upper,
                incl_low,
                incl_high,
            } => {
                let lo = format_string_range_bound(lower.as_deref());
                let hi = format_string_range_bound(upper.as_deref());
                range_repr(key, &lo, &hi, *incl_low, *incl_high)
            }
            Self::Regex { key, pattern } => {
                let display = pattern
                    .as_str()
                    .trim_start_matches('^')
                    .trim_end_matches('$');
                format!("{}:/{}/", dotted_key(key), display)
            }
            Self::Exists { key } => format!("{}: *", dotted_key(key)),
            Self::Null { key } => format!("{}:null", dotted_key(key)),
        }
    }
}

// ─── PyO3-gestützte Extraktionsmethoden ───

impl FilterExpressionInner {
    /// Extrahiert ein FilterExpressionInner aus einem PyFilterExpression oder
    /// einem Python-Objekt mit `expression_type` und Attributen.
    pub fn from_py_object(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Ok(py_expr) = obj.extract::<PyFilterExpression>() {
            return Ok(py_expr.inner.clone());
        }
        let expr_type: String = obj.getattr("expression_type")?.extract()?;
        match expr_type.as_str() {
            "Always" => {
                let value: bool = obj.getattr("value")?.extract()?;
                Ok(FilterExpressionInner::Always { value })
            }
            "Not" => {
                let children_attr = obj.getattr("children")?;
                let child_list = children_attr.downcast::<PyList>()?;
                let child = Self::from_py_object(&child_list.get_item(0)?)?;
                Ok(FilterExpressionInner::Not {
                    child: Box::new(child),
                })
            }
            "And" => {
                let children = Self::extract_children(obj)?;
                Ok(FilterExpressionInner::And { children })
            }
            "Or" => {
                let children = Self::extract_children(obj)?;
                Ok(FilterExpressionInner::Or { children })
            }
            "StringFilterExpression" => {
                let key: Vec<String> = obj.getattr("key")?.extract()?;
                let expected: String = obj.getattr("expected_value")?.extract()?;
                Ok(FilterExpressionInner::String { key, expected })
            }
            "WildcardStringFilterExpression" => {
                let key: Vec<String> = obj.getattr("key")?.extract()?;
                let expected: String = obj.getattr("expected_value")?.extract()?;
                let regex = build_wildcard_regex(&expected)
                    .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
                Ok(FilterExpressionInner::Wildcard {
                    key,
                    expected,
                    regex,
                })
            }
            "SigmaFilterExpression" => {
                let key: Vec<String> = obj.getattr("key")?.extract()?;
                let expected: String = obj.getattr("expected_value")?.extract()?;
                let regex = build_sigma_regex(&expected)
                    .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
                Ok(FilterExpressionInner::Sigma {
                    key,
                    expected,
                    regex,
                })
            }
            "IntegerFilterExpression" => {
                let key: Vec<String> = obj.getattr("key")?.extract()?;
                let expected: String = obj.getattr("expected_value")?.extract()?;
                let val: i64 = expected
                    .parse()
                    .map_err(|_| pyo3::exceptions::PyValueError::new_err(format!("Invalid integer: {}", expected)))?;
                Ok(FilterExpressionInner::Integer {
                    key,
                    expected: val,
                })
            }
            "FloatFilterExpression" => {
                let key: Vec<String> = obj.getattr("key")?.extract()?;
                let expected: String = obj.getattr("expected_value")?.extract()?;
                let val: f64 = expected
                    .parse()
                    .map_err(|_| pyo3::exceptions::PyValueError::new_err(format!("Invalid float: {}", expected)))?;
                Ok(FilterExpressionInner::Float {
                    key,
                    expected: val,
                })
            }
            "IntegerRangeFilterExpression" => {
                let key: Vec<String> = obj.getattr("key")?.extract()?;
                let lower: i64 = obj.getattr("lower")?.extract()?;
                let upper: i64 = obj.getattr("upper")?.extract()?;
                let include_lower: bool = obj.getattr("include_lower")?.extract()?;
                let include_upper: bool = obj.getattr("include_upper")?.extract()?;
                Ok(FilterExpressionInner::IntegerRange {
                    key,
                    lower,
                    upper,
                    incl_low: include_lower,
                    incl_high: include_upper,
                })
            }
            "FloatRangeFilterExpression" => {
                let key: Vec<String> = obj.getattr("key")?.extract()?;
                let lower: f64 = obj.getattr("lower")?.extract()?;
                let upper: f64 = obj.getattr("upper")?.extract()?;
                let include_lower: bool = obj.getattr("include_lower")?.extract()?;
                let include_upper: bool = obj.getattr("include_upper")?.extract()?;
                Ok(FilterExpressionInner::FloatRange {
                    key,
                    lower,
                    upper,
                    incl_low: include_lower,
                    incl_high: include_upper,
                })
            }
            "NumericRangeFilterExpression" => {
                let key: Vec<String> = obj.getattr("key")?.extract()?;
                let lower = numeric_bound_from_py(&obj.getattr("lower")?)?;
                let upper = numeric_bound_from_py(&obj.getattr("upper")?)?;
                let include_lower: bool = obj.getattr("include_lower")?.extract()?;
                let include_upper: bool = obj.getattr("include_upper")?.extract()?;
                Ok(FilterExpressionInner::NumericRange {
                    key,
                    lower,
                    upper,
                    incl_low: include_lower,
                    incl_high: include_upper,
                })
            }
            "StringRangeFilterExpression" => {
                let key: Vec<String> = obj.getattr("key")?.extract()?;
                let lower: Option<String> = obj.getattr("lower")?.extract()?;
                let upper: Option<String> = obj.getattr("upper")?.extract()?;
                let include_lower: bool = obj.getattr("include_lower")?.extract()?;
                let include_upper: bool = obj.getattr("include_upper")?.extract()?;
                Ok(FilterExpressionInner::StringRange {
                    key,
                    lower,
                    upper,
                    incl_low: include_lower,
                    incl_high: include_upper,
                })
            }
            "RegExFilterExpression" => {
                let key: Vec<String> = obj.getattr("key")?.extract()?;
                let raw: String = obj.getattr("expected_value")?.extract()?;
                let normalized = normalize_regex(&raw);
                let compiled = FilterRegex::new(&normalized).map_err(|e| {
                    pyo3::exceptions::PyValueError::new_err(format!("Invalid regex: {}", e))
                })?;
                Ok(FilterExpressionInner::Regex {
                    key,
                    pattern: compiled,
                })
            }
            "Exists" => {
                let key: Vec<String> = obj.getattr("key")?.extract()?;
                Ok(FilterExpressionInner::Exists { key })
            }
            "Null" => {
                let key: Vec<String> = obj.getattr("key")?.extract()?;
                Ok(FilterExpressionInner::Null { key })
            }
            _ => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Unknown expression_type: {}",
                expr_type
            ))),
        }
    }

    fn extract_children(obj: &Bound<'_, PyAny>) -> PyResult<Vec<FilterExpressionInner>> {
        let children = obj.getattr("children")?;
        let child_list = children.downcast::<PyList>()?;
        let mut result = Vec::new();
        for item in child_list.iter() {
            result.push(Self::from_py_object(&item)?);
        }
        Ok(result)
    }
}

// ─── Pure Rust Hilfsfunktionen ───

/// Traversiert ein `serde_json::Value`-Dict entlang eines Key-Pfads.
fn get_json_value<'a>(key: &[String], document: &'a Value) -> Result<&'a Value, MatchError> {
    if key.is_empty() {
        return Err(MatchError::KeyNotFound);
    }
    let mut current = document;
    for segment in key {
        match current {
            Value::Object(map) => {
                current = map.get(segment.as_str()).ok_or(MatchError::KeyNotFound)?;
            }
            _ => return Err(MatchError::TypeMismatch),
        }
    }
    Ok(current)
}

/// Prüft ob ein Pfad in einem serde_json::Value-Dict existiert.
fn path_exists(key: &[String], document: &Value) -> bool {
    if key.is_empty() {
        return false;
    }
    let mut current = document;
    for segment in key {
        match current {
            Value::Object(map) => match map.get(segment.as_str()) {
                Some(child) => current = child,
                None => return false,
            },
            _ => return false,
        }
    }
    true
}

/// Escaped Punkte in Key-Komponenten: ["x.y", "z"] → "x\\.y.z"
fn dotted_key(key: &[String]) -> String {
    key.iter()
        .map(|k| k.replace('.', "\\."))
        .collect::<Vec<_>>()
        .join(".")
}

/// Hilfsfunktion für Range-Repräsentation.
fn range_repr(key: &[String], lower: &str, upper: &str, incl_low: bool, incl_high: bool) -> String {
    let lo = if incl_low { "[" } else { "{" };
    let hi = if incl_high { "]" } else { "}" };
    format!("{}:{}{} TO {}{}", dotted_key(key), lo, lower, upper, hi)
}

/// Formatiert eine String-Range-Grenze für die Repräsentation:
/// offene Grenzen als "*", literale "*" und als Zahl parsebare Grenzen
/// werden in Anführungszeichen gesetzt (wie Python: float() reicht, auch
/// "nan"/"inf").
fn format_string_range_bound(bound: Option<&str>) -> String {
    match bound {
        None => "*".to_string(),
        Some("*") => "\"*\"".to_string(),
        Some(b) => {
            if b.parse::<f64>().is_ok() {
                format!("\"{}\"", b)
            } else {
                b.to_string()
            }
        }
    }
}

// ─── Hilfsfunktionen für Parser (exportiert für lucene.rs) ───

/// Baut ein Regex aus einem Wildcard-Pattern (* → .*, ? → .?).
pub fn build_wildcard_regex(pattern: &str) -> Result<FilterRegex, String> {
    let full = wildcard_pattern(pattern);
    FilterRegex::new(&full).map_err(|e| format!("Invalid regex: {}", e))
}

/// Baut ein case-insensitive Sigma-Regex aus einem Wildcard-Pattern.
pub fn build_sigma_regex(pattern: &str) -> Result<FilterRegex, String> {
    let full = sigma_compiled_pattern(pattern);
    FilterRegex::new(&full).map_err(|e| format!("Invalid regex: {}", e))
}

/// Returns just the wildcard regex pattern string (without compiling).
/// Matches Python's WildcardStringFilterExpression.escaped_expected.
/// Replicates the exact Python algorithm:
/// 1. regex::escape the input
/// 2. Process `\?` patterns, then `\*` patterns
/// 3. Return result (caller wraps with ^...$)
pub fn wildcard_pattern_string(pattern: &str) -> String {
    let escaped = regex::escape(pattern);
    let after_q = replace_wildcard_matches(&escaped, '?', ".?", r"\?");
    replace_wildcard_matches(&after_q, '*', ".*", r"\*")
}

/// Core replacement logic matching Python's `_replace_wildcard`:
/// - finds all `\<wildcard>` patterns (preceded by zero or more backslashes)
/// - for each match:
///   - len - 2 == 0 → wildcard replacement
///   - len - 2 == 2 → match[:-4] + symbol (= keep literal `\<wildcard>`)
///   - len - 2 > 2  → match[:-4] + wildcard_replacement
/// - splits on `(?:\\)*\<wildcard>` and interleaves with replacements
fn replace_wildcard_matches(
    escaped: &str,
    wildcard_char: char,
    wildcard_replacement: &str,
    symbol: &str,
) -> String {
    let find_pattern = format!(r"((?:\\)*\{})", wildcard_char);
    let split_pattern = format!(r"(?:\\)*\{0}", wildcard_char);
    let wc_re = match StdRegex::new(&find_pattern) {
        Ok(r) => r,
        Err(_) => return escaped.to_string(),
    };
    let split_re = match StdRegex::new(&split_pattern) {
        Ok(r) => r,
        Err(_) => return escaped.to_string(),
    };

    // Collect matches
    let mut matches_list: Vec<(usize, usize, String)> = Vec::new();
    for m in wc_re.find_iter(escaped) {
        let s = m.as_str();
        let length = s.len();
        let replacement = match length.checked_sub(2) {
            Some(0) => wildcard_replacement.to_string(),
            Some(2) => {
                let prefix_end = if length >= 4 { length - 4 } else { 0 };
                let prefix = &s[..prefix_end];
                format!("{}{}", prefix, symbol)
            }
            Some(n) if n > 2 => {
                let prefix_end = if length >= 4 { length - 4 } else { 0 };
                let prefix = &s[..prefix_end];
                format!("{}{}", prefix, wildcard_replacement)
            }
            _ => s.to_string(),
        };
        matches_list.push((m.start(), m.end(), replacement));
    }

    if matches_list.is_empty() {
        return escaped.to_string();
    }

    // Split and interleave
    let mut result = String::new();
    let mut last_end = 0;
    let mut match_idx = 0;

    for m in split_re.find_iter(escaped) {
        // Add text before this match
        result.push_str(&escaped[last_end..m.start()]);
        // Add the replacement
        if match_idx < matches_list.len() {
            result.push_str(&matches_list[match_idx].2);
        }
        last_end = m.end();
        match_idx += 1;
    }

    // Add remaining text after last match
    result.push_str(&escaped[last_end..]);
    result
}

/// Returns the full wildcard regex pattern including ^...$ anchors.
/// This matches Python's WildcardStringFilterExpression.escaped_expected.
pub fn wildcard_pattern(pattern: &str) -> String {
    format!("^{}$", wildcard_pattern_string(pattern))
}

/// Returns the full sigma regex pattern including (?i) prefix for compiled matching.
/// Note: Python's SigmaFilterExpression.escaped_expected does NOT include (?i) —
/// it uses re.IGNORECASE flag separately. We store the (?i) prefix internally
/// in the compiled regex, but report the plain pattern for backward compat.
fn sigma_compiled_pattern(pattern: &str) -> String {
    format!("(?i)^{}$", wildcard_pattern_string(pattern))
}

/// Normalisiert ein Regex: fügt ^/$ Anchors hinzu wenn fehlend.
pub fn normalize_regex(regex: &str) -> String {
    let (flags, pattern) = if regex.starts_with("(?") {
        if let Some(end) = regex.find(')') {
            (&regex[..=end], &regex[end + 1..])
        } else {
            ("", regex)
        }
    } else {
        ("", regex)
    };

    let has_caret = pattern.starts_with('^');
    let trailing_dollar_is_anchor = if let Some(c) = pattern.strip_suffix('$') {
        // if even number of backslashes before $, $ is an anchor (not escaped)
        c.chars().rev().take_while(|&ch| ch == '\\').count() % 2 == 0
    } else {
        false
    };

    let mut result = String::from(flags);
    if !has_caret {
        result.push('^');
    }
    result.push_str(pattern);
    if !trailing_dollar_is_anchor {
        result.push('$');
    }
    result
}

// ═══════════════════════════════════════════════════════════
// PyO3 Adapter — Dünne Schicht für Python-API
// ═══════════════════════════════════════════════════════════

/// Einzelne Python-Klasse die alle Expression-Typen repräsentiert.
#[pyclass]
#[derive(Clone)]
pub struct PyFilterExpression {
    pub inner: FilterExpressionInner,
}

#[pymethods]
impl PyFilterExpression {
    /// Typ-Name für isinstance-Äquivalent in Python.
    #[getter]
    fn expression_type(&self) -> &'static str {
        match &self.inner {
            FilterExpressionInner::Always { .. } => "Always",
            FilterExpressionInner::Not { .. } => "Not",
            FilterExpressionInner::And { .. } => "And",
            FilterExpressionInner::Or { .. } => "Or",
            FilterExpressionInner::String { .. } => "StringFilterExpression",
            FilterExpressionInner::Wildcard { .. } => "WildcardStringFilterExpression",
            FilterExpressionInner::Sigma { .. } => "SigmaFilterExpression",
            FilterExpressionInner::Integer { .. } => "IntegerFilterExpression",
            FilterExpressionInner::Float { .. } => "FloatFilterExpression",
            FilterExpressionInner::IntegerRange { .. } => "IntegerRangeFilterExpression",
            FilterExpressionInner::FloatRange { .. } => "FloatRangeFilterExpression",
            FilterExpressionInner::NumericRange { .. } => "NumericRangeFilterExpression",
            FilterExpressionInner::StringRange { .. } => "StringRangeFilterExpression",
            FilterExpressionInner::Regex { .. } => "RegExFilterExpression",
            FilterExpressionInner::Exists { .. } => "Exists",
            FilterExpressionInner::Null { .. } => "Null",
        }
    }

    /// Safe-Matching: Gibt False bei fehlenden Keys/Typfehlern zurück.
    fn matches(&self, py: Python, document: &Bound<'_, PyAny>) -> bool {
        let dict_type = py.get_type::<PyDict>();
        if !document.is_instance(&dict_type).unwrap_or(false) {
            return false;
        }
        match pydict_to_json(document) {
            Ok(json_doc) => self.inner.matches(&json_doc),
            Err(_) => false,
        }
    }

    /// Fallibles Matching — wirft KeyDoesNotExistError bei fehlenden Keys.
    fn does_match(&self, py: Python, document: &Bound<'_, PyAny>) -> PyResult<bool> {
        let json_doc = pydict_to_json(document)
            .map_err(|_| raise_key_does_not_exist_error(py, "Failed to convert document"))?;
        self.inner.does_match(&json_doc).map_err(|e| match e {
            MatchError::KeyNotFound => raise_key_does_not_exist_error(py, "key does not exist"),
            MatchError::TypeMismatch => raise_key_does_not_exist_error(py, "type mismatch"),
        })
    }

    fn __repr__(&self) -> String {
        self.inner.to_repr()
    }

    fn __str__(&self) -> String {
        self.inner.to_repr()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner.to_repr() == other.inner.to_repr()
    }

    // ─── Attribute für KeyBased-Typen ───

    #[getter]
    fn key(&self) -> PyResult<Vec<String>> {
        key_from_inner(&self.inner)
    }

    #[getter]
    fn key_as_dotted_string(&self) -> PyResult<String> {
        let key = key_from_inner(&self.inner)?;
        Ok(dotted_key(&key))
    }

    #[getter]
    fn expected_value(&self) -> PyResult<String> {
        expected_from_inner(&self.inner)
    }

    #[getter]
    fn value(&self) -> PyResult<bool> {
        match &self.inner {
            FilterExpressionInner::Always { value } => Ok(*value),
            _ => Err(pyo3::exceptions::PyAttributeError::new_err(
                "no 'value' attribute",
            )),
        }
    }

    #[getter]
    fn children<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyFilterExpression>>> {
        children_from_inner(&self.inner, py)
    }

    /// Provide iteration over children for __iter__ support (needed by rule_segmenter).
    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let list = PyList::empty(py);
        for child in children_from_inner(&self.inner, py)? {
            list.append(child)?;
        }
        Ok(list)
    }

    /// Provide len() for len() support.
    fn __len__(&self) -> usize {
        match &self.inner {
            FilterExpressionInner::Not { .. } => 1,
            FilterExpressionInner::And { children } => children.len(),
            FilterExpressionInner::Or { children } => children.len(),
            _ => 0,
        }
    }

    /// Internal compiled regex string (for test compatibility).
    #[getter]
    fn _regex(&self) -> PyResult<String> {
        match &self.inner {
            FilterExpressionInner::Regex { pattern, .. } => Ok(pattern.as_str().to_string()),
            _ => Err(pyo3::exceptions::PyAttributeError::new_err(
                "expression has no '_regex'",
            )),
        }
    }

    /// The wildcard/sigma pattern string (for test compatibility).
    /// Returns the pattern with ^...$ anchors, same as Python's escaped_expected.
    #[getter]
    fn escaped_expected(&self) -> PyResult<String> {
        match &self.inner {
            FilterExpressionInner::Wildcard { expected, .. }
            | FilterExpressionInner::Sigma { expected, .. } => {
                Ok(wildcard_pattern(expected))
            }
            _ => Err(pyo3::exceptions::PyAttributeError::new_err(
                "expression has no 'escaped_expected'",
            )),
        }
    }
}

// ─── Python-Dict → serde_json::Value Konverter ───

pub fn pydict_to_json(obj: &Bound<'_, PyAny>) -> PyResult<Value> {
    if let Ok(dict) = obj.downcast::<PyDict>() {
        let mut map = Map::new();
        for (key, value) in dict.iter() {
            let k: String = key.extract()?;
            let v = pyany_to_json(&value)?;
            map.insert(k, v);
        }
        Ok(Value::Object(map))
    } else if let Ok(list) = obj.downcast::<PyList>() {
        let mut arr = Vec::new();
        for item in list.iter() {
            arr.push(pyany_to_json(&item)?);
        }
        Ok(Value::Array(arr))
    } else if obj.is_none() {
        Ok(Value::Null)
    } else if obj.is_instance_of::<pyo3::types::PyBool>() {
        // bool muss vor int geprüft werden (Python: bool ist int-Subklasse)
        Ok(Value::Bool(obj.extract::<bool>()?))
    } else if let Ok(i) = obj.extract::<i64>() {
        Ok(Value::Number(i.into()))
    } else if obj.is_instance_of::<pyo3::types::PyInt>() {
        // Ganzzahl jenseits i64: als String behalten (beliebige Präzision)
        Ok(Value::String(obj.to_string()))
    } else if let Ok(f) = obj.extract::<f64>() {
        Ok(Value::Number(
            serde_json::Number::from_f64(f).unwrap_or(0.into()),
        ))
    } else if let Ok(s) = obj.extract::<String>() {
        Ok(Value::String(s))
    } else {
        Ok(Value::String(obj.to_string()))
    }
}

fn pyany_to_json(obj: &Bound<'_, PyAny>) -> PyResult<Value> {
    if let Ok(dict) = obj.downcast::<PyDict>() {
        let mut map = Map::new();
        for (key, value) in dict.iter() {
            let k: String = key.extract()?;
            let v = pyany_to_json(&value)?;
            map.insert(k, v);
        }
        Ok(Value::Object(map))
    } else if let Ok(list) = obj.downcast::<PyList>() {
        let mut arr = Vec::new();
        for item in list.iter() {
            arr.push(pyany_to_json(&item)?);
        }
        Ok(Value::Array(arr))
    } else if obj.is_none() {
        Ok(Value::Null)
    } else if obj.is_instance_of::<pyo3::types::PyBool>() {
        // bool muss vor int geprüft werden (Python: bool ist int-Subklasse)
        Ok(Value::Bool(obj.extract::<bool>()?))
    } else if let Ok(i) = obj.extract::<i64>() {
        Ok(Value::Number(i.into()))
    } else if obj.is_instance_of::<pyo3::types::PyInt>() {
        // Ganzzahl jenseits i64: als String behalten (beliebige Präzision)
        Ok(Value::String(obj.to_string()))
    } else if let Ok(f) = obj.extract::<f64>() {
        Ok(Value::Number(
            serde_json::Number::from_f64(f).unwrap_or(0.into()),
        ))
    } else if let Ok(s) = obj.extract::<String>() {
        Ok(Value::String(s))
    } else {
        Ok(Value::String(obj.to_string()))
    }
}

// ─── Helper: Key/Value aus Inner extrahieren ───

fn key_from_inner(inner: &FilterExpressionInner) -> PyResult<Vec<String>> {
    match inner {
        FilterExpressionInner::String { key, .. }
        | FilterExpressionInner::Wildcard { key, .. }
        | FilterExpressionInner::Sigma { key, .. }
        | FilterExpressionInner::Integer { key, .. }
        | FilterExpressionInner::Float { key, .. }
        | FilterExpressionInner::IntegerRange { key, .. }
        | FilterExpressionInner::FloatRange { key, .. }
        | FilterExpressionInner::NumericRange { key, .. }
        | FilterExpressionInner::StringRange { key, .. }
        | FilterExpressionInner::Regex { key, .. }
        | FilterExpressionInner::Exists { key }
        | FilterExpressionInner::Null { key } => Ok(key.clone()),
        _ => Err(pyo3::exceptions::PyAttributeError::new_err(
            "expression has no 'key'",
        )),
    }
}

fn expected_from_inner(inner: &FilterExpressionInner) -> PyResult<String> {
    match inner {
        FilterExpressionInner::String { expected, .. }
        | FilterExpressionInner::Wildcard { expected, .. }
        | FilterExpressionInner::Sigma { expected, .. } => Ok(expected.clone()),
        FilterExpressionInner::Integer { expected, .. } => Ok(expected.to_string()),
        FilterExpressionInner::Float { expected, .. } => Ok(expected.to_string()),
        _ => Err(pyo3::exceptions::PyAttributeError::new_err(
            "expression has no 'expected_value'",
        )),
    }
}

fn children_from_inner<'py>(
    inner: &FilterExpressionInner,
    py: Python<'py>,
) -> PyResult<Vec<Bound<'py, PyFilterExpression>>> {
    let children = match inner {
        FilterExpressionInner::Not { child } => vec![child.as_ref().clone()],
        FilterExpressionInner::And { children } => children.clone(),
        FilterExpressionInner::Or { children } => children.clone(),
        _ => {
            return Err(pyo3::exceptions::PyAttributeError::new_err(
                "expression has no 'children'",
            ))
        }
    };
    children
        .into_iter()
        .map(|c| PyFilterExpression { inner: c }.into_pyobject(py))
        .collect()
}

// ═══════════════════════════════════════════════════════════
// Factory-Funktionen (Python-API)
// ═══════════════════════════════════════════════════════════

/// Factory: Always expression.
#[pyfunction]
pub fn filter_expression(value: bool) -> PyFilterExpression {
    PyFilterExpression {
        inner: FilterExpressionInner::Always { value },
    }
}

/// Factory: Not expression.
#[pyfunction]
#[pyo3(signature = (expression,))]
pub fn filter_expression_not(
    expression: &Bound<'_, PyFilterExpression>,
) -> PyResult<PyFilterExpression> {
    let child = expression.borrow().inner.clone();
    Ok(PyFilterExpression {
        inner: FilterExpressionInner::Not {
            child: Box::new(child),
        },
    })
}

/// Factory: And expression.
#[pyfunction]
#[pyo3(signature = (*children,))]
pub fn filter_expression_and(
    children: Vec<Bound<'_, PyFilterExpression>>,
) -> PyResult<PyFilterExpression> {
    let child_inners: Vec<FilterExpressionInner> =
        children.iter().map(|c| c.borrow().inner.clone()).collect();
    Ok(PyFilterExpression {
        inner: FilterExpressionInner::And {
            children: child_inners,
        },
    })
}

/// Factory: Or expression.
#[pyfunction]
#[pyo3(signature = (*children,))]
pub fn filter_expression_or(
    children: Vec<Bound<'_, PyFilterExpression>>,
) -> PyResult<PyFilterExpression> {
    let child_inners: Vec<FilterExpressionInner> =
        children.iter().map(|c| c.borrow().inner.clone()).collect();
    Ok(PyFilterExpression {
        inner: FilterExpressionInner::Or {
            children: child_inners,
        },
    })
}

/// Factory: StringFilterExpression.
#[pyfunction]
pub fn filter_expression_string(key: Vec<String>, expected_value: String) -> PyFilterExpression {
    PyFilterExpression {
        inner: FilterExpressionInner::String {
            key,
            expected: expected_value,
        },
    }
}

/// Factory: WildcardStringFilterExpression.
#[pyfunction]
pub fn filter_expression_wildcard(
    key: Vec<String>,
    expected_value: String,
) -> PyResult<PyFilterExpression> {
    let regex = build_wildcard_regex(&expected_value)
        .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
    Ok(PyFilterExpression {
        inner: FilterExpressionInner::Wildcard {
            key,
            expected: expected_value,
            regex,
        },
    })
}

/// Factory: SigmaFilterExpression (case-insensitive wildcard).
#[pyfunction]
pub fn filter_expression_sigma(
    key: Vec<String>,
    expected_value: String,
) -> PyResult<PyFilterExpression> {
    let regex = build_sigma_regex(&expected_value)
        .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
    Ok(PyFilterExpression {
        inner: FilterExpressionInner::Sigma {
            key,
            expected: expected_value,
            regex,
        },
    })
}

/// Factory: IntegerFilterExpression.
#[pyfunction]
pub fn filter_expression_integer(
    key: Vec<String>,
    expected_value: i64,
) -> PyResult<PyFilterExpression> {
    Ok(PyFilterExpression {
        inner: FilterExpressionInner::Integer {
            key,
            expected: expected_value,
        },
    })
}

/// Factory: FloatFilterExpression.
#[pyfunction]
pub fn filter_expression_float(
    key: Vec<String>,
    expected_value: f64,
) -> PyResult<PyFilterExpression> {
    Ok(PyFilterExpression {
        inner: FilterExpressionInner::Float {
            key,
            expected: expected_value,
        },
    })
}

/// Factory: IntegerRangeFilterExpression.
#[pyfunction]
#[pyo3(signature = (key, lower, upper, include_lower = true, include_upper = true))]
pub fn filter_expression_integer_range(
    key: Vec<String>,
    lower: i64,
    upper: i64,
    include_lower: bool,
    include_upper: bool,
) -> PyResult<PyFilterExpression> {
    if lower > upper {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "Range lower > upper",
        ));
    }
    Ok(PyFilterExpression {
        inner: FilterExpressionInner::IntegerRange {
            key,
            lower,
            upper,
            incl_low: include_lower,
            incl_high: include_upper,
        },
    })
}

/// Factory: FloatRangeFilterExpression.
#[pyfunction]
#[pyo3(signature = (key, lower, upper, include_lower = true, include_upper = true))]
pub fn filter_expression_float_range(
    key: Vec<String>,
    lower: f64,
    upper: f64,
    include_lower: bool,
    include_upper: bool,
) -> PyResult<PyFilterExpression> {
    if !lower.is_finite() || !upper.is_finite() {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "Range boundaries must be finite",
        ));
    }
    if lower > upper {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "Range lower > upper",
        ));
    }
    Ok(PyFilterExpression {
        inner: FilterExpressionInner::FloatRange {
            key,
            lower,
            upper,
            incl_low: include_lower,
            incl_high: include_upper,
        },
    })
}

/// Extrahiert eine optionale numerische Grenze aus einem Python-Objekt
/// (None, int oder float; Fließkommazahlen müssen endlich sein).
fn numeric_bound_from_py(obj: &Bound<'_, PyAny>) -> PyResult<Option<NumericBound>> {
    if obj.is_none() {
        return Ok(None);
    }
    if let Ok(i) = obj.extract::<i64>() {
        return Ok(Some(NumericBound::Int(i.to_string())));
    }
    if obj.is_instance_of::<pyo3::types::PyInt>() {
        // Ganzzahl jenseits i64: beliebige Präzision als String behalten
        return Ok(Some(NumericBound::Int(obj.to_string())));
    }
    if let Ok(f) = obj.extract::<f64>() {
        if !f.is_finite() {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "Range boundaries must be finite",
            ));
        }
        return Ok(Some(NumericBound::Float(f)));
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "Range boundaries must be int, float or None",
    ))
}

/// Factory: NumericRangeFilterExpression.
#[pyfunction]
#[pyo3(signature = (key, lower, upper, include_lower = true, include_upper = true))]
pub fn filter_expression_numeric_range(
    key: Vec<String>,
    lower: Option<&Bound<'_, PyAny>>,
    upper: Option<&Bound<'_, PyAny>>,
    include_lower: bool,
    include_upper: bool,
) -> PyResult<PyFilterExpression> {
    let lower = match lower {
        Some(obj) => numeric_bound_from_py(obj)?,
        None => None,
    };
    let upper = match upper {
        Some(obj) => numeric_bound_from_py(obj)?,
        None => None,
    };
    if let (Some(lo), Some(hi)) = (&lower, &upper) {
        if lo.compare(hi) == std::cmp::Ordering::Greater {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "Range lower > upper",
            ));
        }
    }
    Ok(PyFilterExpression {
        inner: FilterExpressionInner::NumericRange {
            key,
            lower,
            upper,
            incl_low: include_lower,
            incl_high: include_upper,
        },
    })
}

/// Factory: StringRangeFilterExpression.
#[pyfunction]
#[pyo3(signature = (key, lower, upper, include_lower = true, include_upper = true))]
pub fn filter_expression_string_range(
    key: Vec<String>,
    lower: Option<String>,
    upper: Option<String>,
    include_lower: bool,
    include_upper: bool,
) -> PyResult<PyFilterExpression> {
    if let (Some(lo), Some(hi)) = (&lower, &upper) {
        if lo > hi {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "Range lower > upper",
            ));
        }
    }
    Ok(PyFilterExpression {
        inner: FilterExpressionInner::StringRange {
            key,
            lower,
            upper,
            incl_low: include_lower,
            incl_high: include_upper,
        },
    })
}

/// Factory: RegExFilterExpression.
#[pyfunction]
pub fn filter_expression_regex(
    key: Vec<String>,
    regex_pattern: String,
) -> PyResult<PyFilterExpression> {
    let normalized = normalize_regex(&regex_pattern);
    let compiled =
        FilterRegex::new(&normalized).map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid regex: {}", e))
        })?;
    Ok(PyFilterExpression {
        inner: FilterExpressionInner::Regex {
            key,
            pattern: compiled,
        },
    })
}

/// Factory: Exists expression.
#[pyfunction]
pub fn filter_expression_exists(key: Vec<String>) -> PyFilterExpression {
    PyFilterExpression {
        inner: FilterExpressionInner::Exists { key },
    }
}

/// Factory: Null expression.
#[pyfunction]
pub fn filter_expression_null(key: Vec<String>) -> PyFilterExpression {
    PyFilterExpression {
        inner: FilterExpressionInner::Null { key },
    }
}

// ═══════════════════════════════════════════════════════════
// Rust-Unit-Tests (pure Rust, kein GIL nötig)
// ═══════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn always_true_matches() {
        let expr = FilterExpressionInner::Always { value: true };
        let doc = json!({});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn always_false_does_not_match() {
        let expr = FilterExpressionInner::Always { value: false };
        let doc = json!({});
        assert!(!expr.matches(&doc));
    }

    #[test]
    fn not_negates_child() {
        let child = FilterExpressionInner::Always { value: false };
        let expr = FilterExpressionInner::Not {
            child: Box::new(child),
        };
        let doc = json!({});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn and_requires_all_children() {
        let c1 = FilterExpressionInner::Always { value: true };
        let c2 = FilterExpressionInner::Always { value: false };
        let expr = FilterExpressionInner::And {
            children: vec![c1, c2],
        };
        let doc = json!({});
        assert!(!expr.matches(&doc));
    }

    #[test]
    fn or_requires_any_child() {
        let c1 = FilterExpressionInner::Always { value: false };
        let c2 = FilterExpressionInner::Always { value: true };
        let expr = FilterExpressionInner::Or {
            children: vec![c1, c2],
        };
        let doc = json!({});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn string_exact_match() {
        let expr = FilterExpressionInner::String {
            key: vec!["field".into()],
            expected: "expected".into(),
        };
        let doc = json!({"field": "expected"});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn string_list_membership() {
        let expr = FilterExpressionInner::String {
            key: vec!["tags".into()],
            expected: "critical".into(),
        };
        let doc = json!({"tags": ["info", "critical", "warn"]});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn exists_matches_present_key() {
        let expr = FilterExpressionInner::Exists {
            key: vec!["foo".into()],
        };
        let doc = json!({"foo": "bar"});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn exists_does_not_match_missing_key() {
        let expr = FilterExpressionInner::Exists {
            key: vec!["missing".into()],
        };
        let doc = json!({});
        assert!(!expr.matches(&doc));
    }

    #[test]
    fn null_matches_none_value() {
        let expr = FilterExpressionInner::Null {
            key: vec!["field".into()],
        };
        let doc = json!({"field": null});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn integer_exact_match() {
        let expr = FilterExpressionInner::Integer {
            key: vec!["count".into()],
            expected: 42,
        };
        let doc = json!({"count": 42});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn integer_range_inclusive() {
        let expr = FilterExpressionInner::IntegerRange {
            key: vec!["age".into()],
            lower: 18,
            upper: 65,
            incl_low: true,
            incl_high: true,
        };
        let doc = json!({"age": 25});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn integer_range_excludes_out_of_bounds() {
        let expr = FilterExpressionInner::IntegerRange {
            key: vec!["age".into()],
            lower: 18,
            upper: 65,
            incl_low: true,
            incl_high: true,
        };
        let doc = json!({"age": 10});
        assert!(!expr.matches(&doc));
    }

    #[test]
    fn wildcard_star_matches_any() {
        let regex = build_wildcard_regex("foo*bar").unwrap();
        let expr = FilterExpressionInner::Wildcard {
            key: vec!["name".into()],
            expected: "foo*bar".into(),
            regex,
        };
        let doc = json!({"name": "foobar"});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn wildcard_question_mark() {
        let regex = build_wildcard_regex("f?o").unwrap();
        let expr = FilterExpressionInner::Wildcard {
            key: vec!["name".into()],
            expected: "f?o".into(),
            regex,
        };
        let doc = json!({"name": "foo"});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn regex_match() {
        let pattern = FilterRegex::new("^192\\.168\\..*$").unwrap();
        let expr = FilterExpressionInner::Regex {
            key: vec!["ip".into()],
            pattern,
        };
        let doc = json!({"ip": "192.168.0.1"});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn nested_key_access() {
        let expr = FilterExpressionInner::String {
            key: vec!["a".into(), "b".into(), "c".into()],
            expected: "deep".into(),
        };
        let doc = json!({"a": {"b": {"c": "deep"}}});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn missing_key_returns_false() {
        let expr = FilterExpressionInner::String {
            key: vec!["missing".into()],
            expected: "x".into(),
        };
        let doc = json!({});
        assert!(!expr.matches(&doc));
    }

    #[test]
    fn to_repr_always() {
        let expr = FilterExpressionInner::Always { value: true };
        assert_eq!(expr.to_repr(), "*");
    }

    #[test]
    fn to_repr_string() {
        let expr = FilterExpressionInner::String {
            key: vec!["a".into(), "b".into()],
            expected: "val".into(),
        };
        assert_eq!(expr.to_repr(), "a.b:\"val\"");
    }

    #[test]
    fn to_repr_not() {
        let child = FilterExpressionInner::Always { value: true };
        let expr = FilterExpressionInner::Not {
            child: Box::new(child),
        };
        assert_eq!(expr.to_repr(), "NOT (*)");
    }

    #[test]
    fn float_exact_match() {
        let expr = FilterExpressionInner::Float {
            key: vec!["val".into()],
            expected: 3.14,
        };
        let doc = json!({"val": 3.14});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn float_range_inclusive() {
        let expr = FilterExpressionInner::FloatRange {
            key: vec!["temp".into()],
            lower: 18.5,
            upper: 25.0,
            incl_low: true,
            incl_high: true,
        };
        let doc = json!({"temp": 20.0});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn string_range_inclusive() {
        let expr = FilterExpressionInner::StringRange {
            key: vec!["status".into()],
            lower: Some("alpha".into()),
            upper: Some("zulu".into()),
            incl_low: true,
            incl_high: true,
        };
        let doc = json!({"status": "beta"});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn exists_nested_key() {
        let expr = FilterExpressionInner::Exists {
            key: vec!["a".into(), "b".into()],
        };
        let doc = json!({"a": {"b": 42}});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn null_does_not_match_non_null() {
        let expr = FilterExpressionInner::Null {
            key: vec!["field".into()],
        };
        let doc = json!({"field": "not null"});
        assert!(!expr.matches(&doc));
    }

    #[test]
    fn and_mixed() {
        let c1 = FilterExpressionInner::String {
            key: vec!["a".into()],
            expected: "1".into(),
        };
        let c2 = FilterExpressionInner::Exists {
            key: vec!["b".into()],
        };
        let expr = FilterExpressionInner::And {
            children: vec![c1, c2],
        };
        let doc = json!({"a": "1", "b": 42});
        assert!(expr.matches(&doc));
        let doc2 = json!({"a": "1"});
        assert!(!expr.matches(&doc2));
    }

    #[test]
    fn or_mixed() {
        let c1 = FilterExpressionInner::String {
            key: vec!["a".into()],
            expected: "1".into(),
        };
        let c2 = FilterExpressionInner::Exists {
            key: vec!["b".into()],
        };
        let expr = FilterExpressionInner::Or {
            children: vec![c1, c2],
        };
        let doc = json!({"b": 42});
        assert!(expr.matches(&doc));
        let doc2 = json!({});
        assert!(!expr.matches(&doc2));
    }

    #[test]
    fn integer_range_boundary_exclusive() {
        let expr = FilterExpressionInner::IntegerRange {
            key: vec!["age".into()],
            lower: 18,
            upper: 65,
            incl_low: false,
            incl_high: false,
        };
        let doc = json!({"age": 18});
        assert!(!expr.matches(&doc));
        let doc2 = json!({"age": 19});
        assert!(expr.matches(&doc2));
    }

    #[test]
    fn to_repr_exists() {
        let expr = FilterExpressionInner::Exists {
            key: vec!["x".into(), "y".into()],
        };
        assert_eq!(expr.to_repr(), "x.y: *");
    }

    #[test]
    fn to_repr_null() {
        let expr = FilterExpressionInner::Null {
            key: vec!["field".into()],
        };
        assert_eq!(expr.to_repr(), "field:null");
    }

    #[test]
    fn to_repr_and() {
        let c1 = FilterExpressionInner::Exists {
            key: vec!["a".into()],
        };
        let c2 = FilterExpressionInner::Exists {
            key: vec!["b".into()],
        };
        let expr = FilterExpressionInner::And {
            children: vec![c1, c2],
        };
        assert_eq!(expr.to_repr(), "(a: * AND b: *)");
    }

    #[test]
    fn to_repr_or() {
        let c1 = FilterExpressionInner::Exists {
            key: vec!["a".into()],
        };
        let c2 = FilterExpressionInner::Exists {
            key: vec!["b".into()],
        };
        let expr = FilterExpressionInner::Or {
            children: vec![c1, c2],
        };
        assert_eq!(expr.to_repr(), "(a: * OR b: *)");
    }

    #[test]
    fn to_repr_integer_range() {
        let expr = FilterExpressionInner::IntegerRange {
            key: vec!["age".into()],
            lower: 18,
            upper: 65,
            incl_low: true,
            incl_high: true,
        };
        assert_eq!(expr.to_repr(), "age:[18 TO 65]");
    }

    #[test]
    fn to_repr_regex() {
        let pattern = FilterRegex::new("^foo.*bar$").unwrap();
        let expr = FilterExpressionInner::Regex {
            key: vec!["ip".into()],
            pattern,
        };
        assert_eq!(expr.to_repr(), "ip:/foo.*bar/");
    }

    #[test]
    fn to_repr_float() {
        let expr = FilterExpressionInner::Float {
            key: vec!["val".into()],
            expected: 3.14,
        };
        assert_eq!(expr.to_repr(), "val:3.14");
    }

    #[test]
    fn to_repr_integer() {
        let expr = FilterExpressionInner::Integer {
            key: vec!["count".into()],
            expected: 42,
        };
        assert_eq!(expr.to_repr(), "count:42");
    }

    #[test]
    fn sigma_case_insensitive() {
        let regex = build_sigma_regex("foo*bar").unwrap();
        let expr = FilterExpressionInner::Sigma {
            key: vec!["name".into()],
            expected: "foo*bar".into(),
            regex,
        };
        let doc = json!({"name": "FOObar"});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn regex_no_match() {
        let pattern = FilterRegex::new("^foo$").unwrap();
        let expr = FilterExpressionInner::Regex {
            key: vec!["field".into()],
            pattern,
        };
        let doc = json!({"field": "bar"});
        assert!(!expr.matches(&doc));
    }

    #[test]
    fn wildcard_does_not_match_missing_key() {
        let regex = build_wildcard_regex("foo*").unwrap();
        let expr = FilterExpressionInner::Wildcard {
            key: vec!["missing".into()],
            expected: "foo*".into(),
            regex,
        };
        let doc = json!({});
        assert!(!expr.matches(&doc));
    }

    #[test]
    fn string_matches_number_by_string_representation() {
        let expr = FilterExpressionInner::String {
            key: vec!["field".into()],
            expected: "42".into(),
        };
        let doc = json!({"field": 42});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn string_matches_bool_by_string_representation() {
        let expr = FilterExpressionInner::String {
            key: vec!["field".into()],
            expected: "true".into(),
        };
        let doc = json!({"field": true});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn integer_matches_float_with_same_value() {
        let expr = FilterExpressionInner::Integer {
            key: vec!["field".into()],
            expected: 42,
        };
        let doc = json!({"field": 42.0});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn float_matches_integer_with_same_value() {
        let expr = FilterExpressionInner::Float {
            key: vec!["field".into()],
            expected: 42.0,
        };
        let doc = json!({"field": 42});
        assert!(expr.matches(&doc));
    }

    #[test]
    fn integer_range_does_not_match_float() {
        let expr = FilterExpressionInner::IntegerRange {
            key: vec!["field".into()],
            lower: 0,
            upper: 100,
            incl_low: true,
            incl_high: true,
        };
        let doc = json!({"field": 42.5});
        assert!(!expr.matches(&doc));
    }

    #[test]
    fn exists_empty_key() {
        let expr = FilterExpressionInner::Exists { key: vec![] };
        let doc = json!({"foo": "bar"});
        assert!(!expr.matches(&doc));
    }
}
