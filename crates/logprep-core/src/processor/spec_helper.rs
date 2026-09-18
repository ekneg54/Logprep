//! Gemeinsame Helfer fuer `RuleSpec`-Implementierungen (Phase 4b).
//!
//! Enthaelt Regel-Validierung (`validate_required_keys`) und das einmalige,
//! beim `add_rule` durchgefuehrte Aufsplitten von Dotted-Field-Strings
//! (`split_dotted_fields`) — beides wird in Phase 3.5 noch in Python pro
//! Event ausgefuehrt, in Phase 4 nur noch einmal pro Rule.

use serde_json::{Map, Value};

use crate::field::value::get_dotted_field_list;

/// Prueft, dass die angegebenen Pflichtschluessel im Rule-Rohdaten-`Map`
/// vorhanden sind. Ergänzt, nicht ersetzt die attrs-Validatoren der
/// Python-`Rule`-Klasse.
pub fn validate_required_keys(raw: &Map<String, Value>, required: &[&str]) -> Result<(), String> {
    for key in required {
        if !raw.contains_key(*key) {
            return Err(format!("missing required key '{key}'"));
        }
    }
    Ok(())
}

/// Liest eine Liste von Strings aus dem Rule-Rohdaten-`Map`.
pub fn get_string_list(raw: &Map<String, Value>, key: &str) -> Result<Vec<String>, String> {
    match raw.get(key) {
        Some(Value::Array(arr)) => arr
            .iter()
            .map(|v| {
                v.as_str()
                    .map(String::from)
                    .ok_or_else(|| format!("'{key}' must be a list of strings"))
            })
            .collect(),
        Some(_) => Err(format!("'{key}' must be a list of strings")),
        None => Err(format!("missing required key '{key}'")),
    }
}

/// Liest einen optionalen String aus dem Rule-Rohdaten-`Map`.
pub fn get_string(raw: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match raw.get(key) {
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("'{key}' must be a string")),
        None => Ok(None),
    }
}

/// Liest einen optionalen Bool aus dem Rule-Rohdaten-`Map`.
pub fn get_bool(raw: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match raw.get(key) {
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("'{key}' must be a bool")),
        None => Ok(None),
    }
}

/// Splittet Dotted-Field-Strings einmalig beim `add_rule`
/// (verwendet `field::value::get_dotted_field_list`, Phase-1-Wiederverwendung).
pub fn split_dotted_fields(fields: &[String]) -> Vec<Vec<String>> {
    fields.iter().map(|f| get_dotted_field_list(f)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn raw_with(entries: &[(&str, Value)]) -> Map<String, Value> {
        let mut map = Map::new();
        for (k, v) in entries {
            map.insert((*k).to_string(), v.clone());
        }
        map
    }

    #[test]
    fn validate_required_keys_passes_when_present() {
        let raw = raw_with(&[("drop", json!(["a.b"]))]);
        assert!(validate_required_keys(&raw, &["drop"]).is_ok());
    }

    #[test]
    fn validate_required_keys_fails_when_missing() {
        let raw = raw_with(&[]);
        assert!(validate_required_keys(&raw, &["drop"]).is_err());
    }

    #[test]
    fn validate_required_keys_checks_multiple() {
        let raw = raw_with(&[("drop", json!([]))]);
        assert!(validate_required_keys(&raw, &["drop", "drop_full"]).is_err());
        let raw = raw_with(&[("drop", json!([])), ("drop_full", json!(true))]);
        assert!(validate_required_keys(&raw, &["drop", "drop_full"]).is_ok());
    }

    #[test]
    fn get_string_list_ok() {
        let raw = raw_with(&[("drop", json!(["a", "b.c"]))]);
        assert_eq!(
            get_string_list(&raw, "drop").unwrap(),
            vec!["a".to_string(), "b.c".to_string()]
        );
    }

    #[test]
    fn get_string_list_rejects_non_strings() {
        let raw = raw_with(&[("drop", json!(["a", 42]))]);
        assert!(get_string_list(&raw, "drop").is_err());
    }

    #[test]
    fn get_string_list_missing_key() {
        let raw = raw_with(&[]);
        assert!(get_string_list(&raw, "drop").is_err());
    }

    #[test]
    fn get_string_and_bool_optional() {
        let raw = raw_with(&[("target", json!("t")), ("flag", json!(true))]);
        assert_eq!(get_string(&raw, "target").unwrap().unwrap(), "t");
        assert_eq!(get_string(&raw, "missing").unwrap(), None);
        assert_eq!(get_bool(&raw, "flag").unwrap(), Some(true));
        assert_eq!(get_bool(&raw, "missing").unwrap(), None);
        let raw = raw_with(&[("target", json!(3))]);
        assert!(get_string(&raw, "target").is_err());
    }

    #[test]
    fn split_dotted_fields_escapes() {
        let fields = vec!["a.b".to_string(), r"lone\.field.c".to_string()];
        let parts = split_dotted_fields(&fields);
        assert_eq!(parts[0], vec!["a".to_string(), "b".to_string()]);
        assert_eq!(parts[1], vec!["lone.field".to_string(), "c".to_string()]);
    }

    #[test]
    fn split_dotted_fields_single() {
        let fields = vec![String::from("plain")];
        assert_eq!(
            split_dotted_fields(&fields),
            vec![vec!["plain".to_string()]]
        );
    }
}
