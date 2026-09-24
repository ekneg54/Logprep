//! Pure-Rust field helpers operating on `serde_json::Value` (Phase 4a).
//!
//! These functions mirror the semantics of the Phase-1 Python `logprep.util.helper`
//! functions byte-for-byte where the event is represented as a plain JSON value.
//! They are consumed by the processor `RuleSpec` implementations and — via thin
//! wrappers in `field::py` — by the legacy Python API.

use serde_json::{Map, Value};

/// Error raised when a field cannot be written because a sub- or target field
/// exists and cannot be extended. Mirrors Python's `FieldExistsWarning`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FieldExistsError {
    /// Ordered list of dotted field names that could not be written.
    pub skipped_fields: Vec<String>,
}

// =============================================================================
// Parsing
// =============================================================================

/// Splits a dotted field string into its components, honouring backslash
/// escaping of `.` and `\` (mirrors `helper.get_dotted_field_list`).
pub fn get_dotted_field_list(dotted_field: &str) -> Vec<String> {
    if !dotted_field.contains('\\') {
        return dotted_field.split('.').map(String::from).collect();
    }

    let mut result = Vec::new();
    let mut char_buffer = String::new();
    let mut chars = dotted_field.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '.' => result.push(std::mem::take(&mut char_buffer)),
            '\\' => match chars.next() {
                Some('.') => char_buffer.push('.'),
                Some('\\') => char_buffer.push('\\'),
                Some(next) => {
                    char_buffer.push('\\');
                    char_buffer.push(next);
                }
                None => char_buffer.push('\\'),
            },
            _ => char_buffer.push(c),
        }
    }
    result.push(char_buffer);
    result
}

/// Combines a field list into a dotted field, escaping dots in field names.
pub fn field_list_to_dotted_field(field_list: &[String]) -> String {
    field_list
        .iter()
        .map(|field| field.replace('.', "\\."))
        .collect::<Vec<_>>()
        .join(".")
}

/// Combines dotted fields without escaping.
pub fn join_dotted_fields(dotted_fields: &[String]) -> String {
    dotted_fields.join(".")
}

/// Normalizes a Python-style list index (supporting negatives) to a `usize`.
fn normalize_index(idx: isize, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let len_i = len as isize;
    let mut i = idx;
    if i < 0 {
        i += len_i;
    }
    if i < 0 || i >= len_i {
        None
    } else {
        Some(i as usize)
    }
}

/// Python list-slicing semantics matching `slice.indices(length)` plus the
/// `start < stop` (resp. `start > stop`) iteration bounds of CPython.
fn python_slice(
    arr: &[Value],
    start: Option<isize>,
    stop: Option<isize>,
    step: isize,
) -> Vec<Value> {
    if step == 0 {
        return Vec::new();
    }
    let len = arr.len() as isize;
    let (start, stop) = if step > 0 {
        let s = match start {
            Some(s) if s < 0 => (s + len).max(0),
            Some(s) => s.min(len),
            None => 0,
        };
        let e = match stop {
            Some(e) if e < 0 => (e + len).max(0),
            Some(e) => e.min(len),
            None => len,
        };
        (s, e)
    } else {
        let s = match start {
            Some(s) if s < 0 => (s + len).max(-1),
            Some(s) => s.min(len - 1),
            None => len - 1,
        };
        let e = match stop {
            Some(e) if e < 0 => (e + len).max(-1),
            Some(e) => e.min(len - 1),
            None => -1,
        };
        (s, e)
    };
    let mut result = Vec::new();
    let mut i = start;
    if step > 0 {
        while i < stop {
            if i >= 0 && i < len {
                result.push(arr[i as usize].clone());
            }
            i += step;
        }
    } else {
        while i > stop {
            if i >= 0 && i < len {
                result.push(arr[i as usize].clone());
            }
            i += step;
        }
    }
    result
}

/// Resolves one path segment against the current container. Handles list
/// indices and slices.
enum Step {
    Index(isize),
    Slice(Option<isize>, Option<isize>, isize),
}

fn step_from_segment(segment: &str) -> Option<Step> {
    if segment.contains(':') {
        let parts: Vec<&str> = segment.split(':').collect();
        if parts.len() > 3 {
            return None;
        }
        let parse = |s: &str| -> Option<Option<isize>> {
            if s.is_empty() {
                Some(None)
            } else {
                s.parse::<isize>().map(Some).ok()
            }
        };
        let start = parse(parts[0])?;
        let stop = parse(parts.get(1).copied().unwrap_or(""))?;
        let step = parts
            .get(2)
            .copied()
            .unwrap_or("")
            .parse::<isize>()
            .ok()
            .unwrap_or(1);
        if step == 0 {
            None
        } else {
            Some(Step::Slice(start, stop, step))
        }
    } else {
        match segment.parse::<isize>() {
            Ok(idx) => Some(Step::Index(idx)),
            Err(_) => None,
        }
    }
}

// =============================================================================
// Read
// =============================================================================

/// Returns a reference to the value at the given field path, or `None` if the
/// field is missing. Dict keys and list indices are supported; slices on lists
/// are not (use `get_dotted_field_value_owned` for those).
pub fn get_at<'a>(event: &'a Value, parts: &[String]) -> Option<&'a Value> {
    let mut current = event;
    for part in parts {
        match current {
            Value::Object(map) => current = map.get(part)?,
            Value::Array(arr) => {
                if part.contains(':') {
                    return None;
                }
                let idx = part.parse::<isize>().ok()?;
                current = arr.get(normalize_index(idx, arr.len())?)?;
            }
            _ => return None,
        }
    }
    Some(current)
}

/// Returns a borrowed value for a dotted field (no slice support).
pub fn get_dotted_field_value<'a>(event: &'a Value, key: &[String]) -> Option<&'a Value> {
    get_at(event, key)
}

/// Returns an owned value, supporting list slicing in the path.
pub fn get_dotted_field_value_owned(event: &Value, parts: &[String]) -> Option<Value> {
    let mut current = event.clone();
    for part in parts {
        match current {
            Value::Object(mut map) => {
                current = map.remove(part)?;
            }
            Value::Array(arr) => match step_from_segment(part) {
                Some(Step::Index(idx)) => {
                    let i = normalize_index(idx, arr.len())?;
                    current = arr.get(i)?.clone();
                }
                Some(Step::Slice(start, stop, step)) => {
                    let sliced = python_slice(&arr, start, stop, step);
                    current = Value::Array(sliced);
                }
                _ => return None,
            },
            _ => return None,
        }
    }
    Some(current)
}

/// Returns all requested source field values as an ordered list of
/// (dotted field name, value). Missing fields map to `None`.
pub fn get_source_fields_dict<'a>(
    event: &'a Value,
    source_fields: &[String],
) -> Vec<(String, Option<&'a Value>)> {
    source_fields
        .iter()
        .map(|field| {
            let parts = get_dotted_field_list(field);
            (field.clone(), get_dotted_field_value(event, &parts))
        })
        .collect()
}

// =============================================================================
// Existence
// =============================================================================

/// Checks whether a dotted field exists. With `allow_none=false` a `null`
/// value counts as missing.
pub fn has_dotted_field(event: &Value, key: &[String], allow_none: bool) -> bool {
    match get_at(event, key) {
        None => false,
        Some(value) => allow_none || !value.is_null(),
    }
}

// =============================================================================
// Pop
// =============================================================================

/// Removes and returns the value at the given path. With `drop_empty=true`
/// empty intermediate dicts are removed as well (recursion mirrors
/// `helper.pop_field_value_and_drop_empty`).
pub fn pop_dotted_field_value(
    event: &mut Value,
    parts: &[String],
    drop_empty: bool,
) -> Option<Value> {
    if parts.is_empty() {
        return None;
    }
    if drop_empty {
        pop_and_drop_empty(event, parts)
    } else {
        pop_exact(event, parts)
    }
}

fn pop_and_drop_empty(event: &mut Value, parts: &[String]) -> Option<Value> {
    match event {
        Value::Object(map) => {
            let key = &parts[0];
            if !map.contains_key(key) {
                return None;
            }
            if parts.len() == 1 {
                return map.remove(key);
            }
            let child = map.get_mut(key)?;
            let value = pop_and_drop_empty(child, &parts[1..]);
            // Leere Eltern-Dicts werden auch dann entfernt, wenn der Leaf
            // nicht existiert (entspricht dem bedingungslosen
            // `if not sub_dict[next_key]: del ...` aus helper.py).
            if let Value::Object(c) = child
                && c.is_empty()
            {
                map.remove(key);
            }
            value
        }
        _ => None,
    }
}

/// Resolves one path segment against the container in place; list indices are
/// supported alongside dict keys (mirrors `helper.get_item` used by
/// `pop_field_value`'s parent traversal).
fn get_mut_item<'a>(current: &'a mut Value, key: &str) -> Option<&'a mut Value> {
    match current {
        Value::Object(map) => map.get_mut(key),
        Value::Array(arr) => {
            let idx = key.parse::<isize>().ok()?;
            let i = normalize_index(idx, arr.len())?;
            arr.get_mut(i)
        }
        _ => None,
    }
}

/// Removes the exact field without dropping empty parents. The final key is
/// only ever removed from a dict (mirrors `helper.pop_field_value`).
fn pop_exact(event: &mut Value, parts: &[String]) -> Option<Value> {
    if parts.len() == 1 {
        return match event {
            Value::Object(map) => map.remove(&parts[0]),
            _ => None,
        };
    }
    let last = parts.last().unwrap();
    let parents = &parts[..parts.len() - 1];
    let mut current = event;
    for part in parents {
        current = get_mut_item(current, part)?;
    }
    match current {
        Value::Object(map) => map.remove(last),
        _ => None,
    }
}

// =============================================================================
// Write
// =============================================================================

/// Traverses (creating plain dicts) and returns a mutable ref to the parent
/// container at `parents`. Fails if an intermediate key exists but is not a
/// dict (unless `overwrite` allows replacing non-dict intermediates with a
/// fresh dict — mirrors `add_and_overwrite_key`).
fn add_and_overwrite_key<'a>(event: &'a mut Value, key: &str) -> Option<&'a mut Value> {
    match event {
        Value::Object(map) => {
            if !map.get(key).is_some_and(Value::is_object) {
                map.insert(key.to_string(), Value::Object(Map::new()));
            }
            map.get_mut(key)
        }
        _ => None,
    }
}

fn add_and_not_overwrite_key<'a>(event: &'a mut Value, key: &str) -> Result<&'a mut Value, ()> {
    match event {
        Value::Object(map) => {
            if map.contains_key(key) {
                if map.get(key).is_some_and(|v| v.is_object()) {
                    Ok(map.get_mut(key).unwrap())
                } else {
                    Err(())
                }
            } else {
                map.insert(key.to_string(), Value::Object(Map::new()));
                Ok(map.get_mut(key).unwrap())
            }
        }
        _ => Err(()),
    }
}

fn field_exists_error(parts: &[String]) -> FieldExistsError {
    FieldExistsError {
        skipped_fields: vec![field_list_to_dotted_field(parts)],
    }
}

/// Adds a single field to the event, mirroring `helper.add_field_to`.
pub fn add_field_to(
    event: &mut Value,
    field_name: &[String],
    content: &Value,
    merge_with_target: bool,
    overwrite_target: bool,
) -> Result<(), FieldExistsError> {
    if field_name.is_empty() {
        return Err(field_exists_error(field_name));
    }
    let target_key = field_name.last().unwrap().clone();
    let parents = &field_name[..field_name.len() - 1];

    if overwrite_target {
        let mut current = event;
        for part in parents {
            current = add_and_overwrite_key(current, part)
                .ok_or_else(|| field_exists_error(field_name))?;
        }
        if let Value::Object(map) = current {
            map.insert(target_key, content.clone());
        } else {
            return Err(field_exists_error(field_name));
        }
        return Ok(());
    }

    let mut current = event;
    for part in parents {
        current =
            add_and_not_overwrite_key(current, part).map_err(|_| field_exists_error(field_name))?;
    }

    match current {
        Value::Object(map) => match map.get_mut(&target_key) {
            None => {
                map.insert(target_key, content.clone());
            }
            Some(existing) if existing.is_null() => {
                *existing = content.clone();
            }
            Some(existing) => {
                if !merge_with_target {
                    return Err(field_exists_error(field_name));
                }
                if existing.is_object() && content.is_object() {
                    let c = content.as_object().unwrap();
                    existing
                        .as_object_mut()
                        .unwrap()
                        .extend(c.iter().map(|(k, v)| (k.clone(), v.clone())));
                } else if existing.is_array() && content.is_array() {
                    existing
                        .as_array_mut()
                        .unwrap()
                        .extend(content.as_array().unwrap().iter().cloned());
                } else if existing.is_array() {
                    existing.as_array_mut().unwrap().push(content.clone());
                } else if content.is_array() {
                    let mut new_arr = vec![existing.clone()];
                    new_arr.extend(content.as_array().unwrap().iter().cloned());
                    map.insert(target_key, Value::Array(new_arr));
                } else {
                    if !overwrite_target {
                        return Err(field_exists_error(field_name));
                    }
                    let new_arr = vec![existing.clone(), content.clone()];
                    map.insert(target_key, Value::Array(new_arr));
                }
            }
        },
        _ => return Err(field_exists_error(field_name)),
    }
    Ok(())
}

/// Single-field add with silent failure; returns the first skipped field name.
pub fn add_field_to_silent_fail(
    event: &mut Value,
    field_name: &[String],
    content: &Value,
    merge_with_target: bool,
    overwrite_target: bool,
) -> Option<String> {
    match add_field_to(
        event,
        field_name,
        content,
        merge_with_target,
        overwrite_target,
    ) {
        Ok(()) => None,
        Err(err) => err.skipped_fields.first().cloned(),
    }
}

/// Adds multiple dotted fields, mirroring `helper.add_fields_to`. When
/// `skip_none` is set, `Null` values are dropped; multi-field failures are
/// aggregated into a single `FieldExistsError`. The slice preserves insertion
/// order (Python dicts are ordered), which determines `skipped_fields` order.
pub fn add_fields_to(
    event: &mut Value,
    fields: &[(String, Value)],
    merge_with_target: bool,
    overwrite_target: bool,
    skip_none: bool,
) -> Result<(), FieldExistsError> {
    let filtered: Vec<(Vec<String>, Value)> = fields
        .iter()
        .filter(|(_, v)| !(skip_none && v.is_null()))
        .map(|(k, v)| (get_dotted_field_list(k), v.clone()))
        .collect();

    if filtered.len() == 1 {
        let (parts, content) = &filtered[0];
        return add_field_to(event, parts, content, merge_with_target, overwrite_target);
    }

    let mut unsuccessful = Vec::new();
    for (parts, content) in &filtered {
        if let Some(skipped) =
            add_field_to_silent_fail(event, parts, content, merge_with_target, overwrite_target)
        {
            unsuccessful.push(skipped);
        }
    }
    if !unsuccessful.is_empty() {
        return Err(FieldExistsError {
            skipped_fields: unsuccessful,
        });
    }
    Ok(())
}

/// `append_as_list` equivalent: add fields merging with an existing target.
pub fn append_as_list(
    event: &mut Value,
    fields: &[(String, Value)],
) -> Result<(), FieldExistsError> {
    add_fields_to(event, fields, true, false, true)
}

/// `add_and_overwrite` equivalent for a single dotted field.
pub fn add_and_overwrite(
    event: &mut Value,
    field_name: &[String],
    content: &Value,
) -> Result<(), FieldExistsError> {
    add_field_to(event, field_name, content, false, true)
}

/// Python `str()` of a scalar value (used by dissector append concatenation).
fn value_to_python_str(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(b) => {
            if *b {
                "True".to_string()
            } else {
                "False".to_string()
            }
        }
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// `helper.append` equivalent: concatenates `content` to an existing scalar
/// target separated by `separator`, or appends to a list target.
pub fn append(
    event: &mut Value,
    target_field: &[String],
    separator: &str,
    content: &Value,
) -> Result<(), FieldExistsError> {
    let dotted = field_list_to_dotted_field(target_field);
    let target_value = get_at(event, target_field).cloned();
    match target_value {
        Some(Value::Array(_)) => {
            let fields = vec![(dotted, content.clone())];
            append_as_list(event, &fields)
        }
        other => {
            let prefix = match other {
                Some(v) => value_to_python_str(&v),
                None => String::new(),
            };
            let merged = format!("{prefix}{separator}{}", value_to_python_str(content));
            add_and_overwrite(event, target_field, &Value::String(merged))
        }
    }
}

// =============================================================================
// Templates
// =============================================================================

/// Substitutes `${<field>}` placeholders in `template` with the values from
/// `data` (in iteration order). Mirrors `helper.resolve_template` with the
/// default Python `str` serializer unless `json_mode` requests compact JSON
/// serialization (calculator).
pub fn resolve_template(template: &str, data: &[(String, Value)], json_mode: bool) -> String {
    let mut result = template.to_string();
    for (key, value) in data {
        let needle = format!("${{{key}}}");
        let replacement = if json_mode {
            serde_json::to_string(value).unwrap_or_default()
        } else {
            match value {
                Value::Null => "None".to_string(),
                Value::Bool(b) => {
                    if *b {
                        "True".to_string()
                    } else {
                        "False".to_string()
                    }
                }
                Value::Number(n) => n.to_string(),
                Value::String(s) => s.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            }
        };
        result = result.replace(&needle, &replacement);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn get_dotted_field_list_simple() {
        assert_eq!(get_dotted_field_list("a.b.c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn get_dotted_field_list_escaped() {
        assert_eq!(
            get_dotted_field_list(r"dotted\.field"),
            vec!["dotted.field"]
        );
        assert_eq!(
            get_dotted_field_list(r"dotted\\.field"),
            vec![r"dotted\", "field"]
        );
        assert_eq!(get_dotted_field_list("simple"), vec!["simple"]);
        assert_eq!(get_dotted_field_list(r"field\\"), vec![r"field\"]);
    }

    #[test]
    fn field_list_to_dotted_field_works() {
        assert_eq!(
            field_list_to_dotted_field(&["a".into(), "b".into(), "c".into()]),
            "a.b.c"
        );
        assert_eq!(
            field_list_to_dotted_field(&["x.y".into(), "z".into()]),
            r"x\.y.z"
        );
    }

    #[test]
    fn join_dotted_fields_works() {
        assert_eq!(join_dotted_fields(&["x.y".into(), "z".into()]), "x.y.z");
    }

    #[test]
    fn get_at_nested_and_array_index() {
        let event = json!({"a": {"b": [10, 20, 30]}, "c": "x"});
        assert_eq!(
            get_dotted_field_value(&event, &["a".into(), "b".into(), "1".into()]),
            Some(&Value::from(20))
        );
        assert_eq!(
            get_dotted_field_value(&event, &["a".into(), "b".into(), "-1".into()]),
            Some(&Value::from(30))
        );
        assert_eq!(
            get_dotted_field_value(&event, &["a".into(), "missing".into()]),
            None
        );
        assert_eq!(
            get_dotted_field_value(&event, &["c".into()]),
            Some(&Value::from("x"))
        );
    }

    #[test]
    fn get_at_out_of_range_is_missing() {
        let event = json!({"a": [1, 2]});
        assert_eq!(
            get_dotted_field_value(&event, &["a".into(), "5".into()]),
            None
        );
        assert_eq!(
            get_dotted_field_value(&event, &["a".into(), "3".into()]),
            None
        );
    }

    #[test]
    fn get_owned_supports_slices() {
        let event = json!({"a": [0, 1, 2, 3, 4, 5]});
        assert_eq!(
            get_dotted_field_value_owned(&event, &["a".into(), "1:4".into()]),
            Some(json!([1, 2, 3]))
        );
        assert_eq!(
            get_dotted_field_value_owned(&event, &["a".into(), "::2".into()]),
            Some(json!([0, 2, 4]))
        );
        assert_eq!(
            get_dotted_field_value_owned(&event, &["a".into(), "-2:".into()]),
            Some(json!([4, 5]))
        );
        assert_eq!(
            get_dotted_field_value_owned(&event, &["a".into(), "5:1:-1".into()]),
            Some(json!([5, 4, 3, 2]))
        );
    }

    #[test]
    fn has_dotted_field_none_semantics() {
        let event = json!({"a": null, "b": 1});
        assert!(has_dotted_field(&event, &["a".into()], true));
        assert!(!has_dotted_field(&event, &["a".into()], false));
        assert!(has_dotted_field(&event, &["b".into()], false));
        assert!(!has_dotted_field(&event, &["c".into()], true));
    }

    #[test]
    fn pop_dotted_field_value_simple() {
        let mut event = json!({"a": {"b": 1, "c": 2}});
        let popped = pop_dotted_field_value(&mut event, &["a".into(), "b".into()], true);
        assert_eq!(popped, Some(Value::from(1)));
        assert_eq!(event, json!({"a": {"c": 2}}));
    }

    #[test]
    fn pop_drops_empty_parents() {
        let mut event = json!({"a": {"b": {"only": 1}}, "keep": "x"});
        let popped =
            pop_dotted_field_value(&mut event, &["a".into(), "b".into(), "only".into()], true);
        assert_eq!(popped, Some(Value::from(1)));
        assert_eq!(event, json!({"keep": "x"}));
    }

    #[test]
    fn pop_not_drop_empty_keeps_parents() {
        let mut event = json!({"a": {"b": {"only": 1}}});
        let popped =
            pop_dotted_field_value(&mut event, &["a".into(), "b".into(), "only".into()], false);
        assert_eq!(popped, Some(Value::from(1)));
        assert_eq!(event, json!({"a": {"b": {}}}));
    }

    #[test]
    fn pop_missing_fields_return_none() {
        let mut event = json!({"a": 1});
        assert_eq!(
            pop_dotted_field_value(&mut event, &["x".into(), "y".into()], true),
            None
        );
        assert_eq!(event, json!({"a": 1}));
    }

    #[test]
    fn pop_from_list_is_missing() {
        let mut event = json!({"a": [10, 20, 30]});
        assert_eq!(
            pop_dotted_field_value(&mut event, &["a".into(), "1".into()], true),
            None
        );
        assert_eq!(event, json!({"a": [10, 20, 30]}));
    }

    #[test]
    fn add_field_to_creates_dotted_target() {
        let mut event = json!({});
        let parts = get_dotted_field_list("a.b.c");
        add_field_to(&mut event, &parts, &Value::from(5), false, false).unwrap();
        assert_eq!(event, json!({"a": {"b": {"c": 5}}}));
    }

    #[test]
    fn add_field_to_overwrite_scalar_parent() {
        let mut event = json!({"a": "scalar"});
        let parts = get_dotted_field_list("a.b");
        add_field_to(&mut event, &parts, &Value::from(5), false, true).unwrap();
        assert_eq!(event, json!({"a": {"b": 5}}));
    }

    #[test]
    fn add_field_to_field_exists_error() {
        let mut event = json!({"a": {"b": 1}});
        let parts = get_dotted_field_list("a.b");
        let err = add_field_to(&mut event, &parts, &Value::from(2), false, false).unwrap_err();
        assert_eq!(err.skipped_fields, vec!["a.b".to_string()]);
        assert_eq!(event, json!({"a": {"b": 1}}));
    }

    #[test]
    fn add_field_to_merge_lists() {
        let mut event = json!({"a": {"b": [1, 2]}});
        let parts = get_dotted_field_list("a.b");
        add_field_to(&mut event, &parts, &json!([3, 4]), true, false).unwrap();
        assert_eq!(event, json!({"a": {"b": [1, 2, 3, 4]}}));
    }

    #[test]
    fn add_field_to_merge_dicts() {
        let mut event = json!({"a": {"b": {"x": 1}}});
        let parts = get_dotted_field_list("a.b");
        add_field_to(&mut event, &parts, &json!({"y": 2}), true, false).unwrap();
        assert_eq!(event, json!({"a": {"b": {"x": 1, "y": 2}}}));
    }

    #[test]
    fn add_field_to_wraps_existing_scalar_with_list_content() {
        let mut event = json!({"a": {"b": "existing"}});
        let parts = get_dotted_field_list("a.b");
        add_field_to(&mut event, &parts, &json!(["new"]), true, false).unwrap();
        assert_eq!(event, json!({"a": {"b": ["existing", "new"]}}));
    }

    #[test]
    fn add_field_to_scalar_scalar_merge_raises_warning() {
        let mut event = json!({"a": {"b": "existing"}});
        let parts = get_dotted_field_list("a.b");
        let err = add_field_to(&mut event, &parts, &Value::from("new"), true, false).unwrap_err();
        assert_eq!(err.skipped_fields, vec!["a.b".to_string()]);
        assert_eq!(event, json!({"a": {"b": "existing"}}));
    }

    #[test]
    fn add_fields_to_multi_field_aggregates_skipped() {
        let mut event = json!({"a": 1, "c": {"nested": "x"}});
        let fields = vec![
            ("a".to_string(), Value::from(2)),
            ("b".to_string(), Value::from(3)),
            ("c.nested".to_string(), Value::from("y")),
        ];
        let err = add_fields_to(&mut event, &fields, true, false, true).unwrap_err();
        assert_eq!(
            err.skipped_fields,
            vec!["a".to_string(), "c.nested".to_string()]
        );
        // erfolgreiche Felder wurden trotzdem geschrieben
        assert_eq!(event.get("b"), Some(&Value::from(3)));
    }

    #[test]
    fn add_fields_to_skips_none_values() {
        let mut event = json!({});
        let fields = vec![
            ("keep".to_string(), Value::from("v")),
            ("skip".to_string(), Value::Null),
        ];
        add_fields_to(&mut event, &fields, false, false, true).unwrap();
        assert_eq!(event, json!({"keep": "v"}));
    }

    #[test]
    fn add_fields_to_keeps_none_when_skip_none_false() {
        let mut event = json!({});
        let fields = vec![("a".to_string(), Value::Null)];
        add_fields_to(&mut event, &fields, false, false, false).unwrap();
        assert_eq!(event, json!({"a": null}));
    }

    #[test]
    fn add_field_to_silent_fail_returns_first_skipped() {
        let mut event = json!({"a": 1});
        let parts = get_dotted_field_list("a");
        assert_eq!(
            add_field_to_silent_fail(&mut event, &parts, &Value::from(9), false, false),
            Some("a".to_string())
        );
        let parts = get_dotted_field_list("b");
        assert_eq!(
            add_field_to_silent_fail(&mut event, &parts, &Value::from(9), false, false),
            None
        );
        assert_eq!(event, json!({"a": 1, "b": 9}));
    }

    #[test]
    fn get_source_fields_dict_maps_missing_to_none() {
        let event = json!({"a": 1});
        let fields = vec!["a".to_string(), "b.c".to_string()];
        let result = get_source_fields_dict(&event, &fields);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "a");
        assert_eq!(result[0].1, Some(&Value::from(1)));
        assert_eq!(result[1].0, "b.c");
        assert!(result[1].1.is_none());
    }

    #[test]
    fn resolve_template_string_mode() {
        let data = vec![("name".to_string(), Value::String("world".into()))];
        assert_eq!(
            resolve_template("hello ${name}!", &data, false),
            "hello world!"
        );
    }

    #[test]
    fn resolve_template_json_mode() {
        let data = vec![
            ("a".to_string(), Value::from(3)),
            ("b".to_string(), Value::String("x".into())),
            ("c".to_string(), Value::Null),
        ];
        assert_eq!(
            resolve_template("${a} ${b} ${c}", &data, true),
            "3 \"x\" null"
        );
    }

    #[test]
    fn resolve_template_keeps_unknown() {
        let data: Vec<(String, Value)> = vec![("a".to_string(), Value::from(1))];
        assert_eq!(
            resolve_template("${a} ${unknown}", &data, true),
            "1 ${unknown}"
        );
    }

    #[test]
    fn append_scalar_uses_separator() {
        let mut event = json!({"t": "start"});
        append(&mut event, &["t".into()], " ", &Value::String("end".into())).unwrap();
        assert_eq!(event, json!({"t": "start end"}));
    }

    #[test]
    fn append_missing_target_includes_separator() {
        let mut event = json!({});
        append(&mut event, &["t".into()], " ", &Value::String("end".into())).unwrap();
        assert_eq!(event, json!({"t": " end"}));
    }

    #[test]
    fn append_to_list_extends() {
        let mut event = json!({"t": ["a"]});
        append(&mut event, &["t".into()], "", &Value::String("b".into())).unwrap();
        assert_eq!(event, json!({"t": ["a", "b"]}));
    }

    #[test]
    fn add_and_overwrite_works() {
        let mut event = json!({"a": 1});
        add_and_overwrite(&mut event, &["a".into()], &Value::from(2)).unwrap();
        assert_eq!(event, json!({"a": 2}));
    }

    #[test]
    fn pop_drop_empty_ignores_list_parents() {
        let mut event = json!({"a": [[]], "b": 1});
        let popped =
            pop_dotted_field_value(&mut event, &["a".into(), "0".into(), "0".into()], true);
        assert_eq!(popped, None);
        assert_eq!(event, json!({"a": [[]], "b": 1}));
    }

    #[test]
    fn escaped_dot_lookup_works() {
        let event = json!({"a.b": {"c": 1}});
        let parts = get_dotted_field_list(r"a\.b.c");
        assert_eq!(
            get_dotted_field_value(&event, &parts),
            Some(&Value::from(1))
        );
    }

    #[test]
    fn dict_keys_with_colons_are_plain_lookups() {
        // py.rs get_item prueft Dict vor Slice — Schluessel mit ':' bleiben
        // normale Dict-Zugriffe (entspricht Python-Verhalten).
        let event = json!({"time:raw": 5, "a": {"b:c": "x"}});
        assert_eq!(
            get_dotted_field_value(&event, &["time:raw".into()]),
            Some(&Value::from(5))
        );
        assert_eq!(
            get_dotted_field_value(&event, &["a".into(), "b:c".into()]),
            Some(&Value::from("x"))
        );
        assert!(has_dotted_field(&event, &["time:raw".into()], true));
    }

    #[test]
    fn colon_only_slices_on_arrays() {
        let event = json!({"a": [1, 2, 3]});
        // Array + Slice-Segment: geliehen Rueckgabe nicht moeglich → None
        // (dafuer existiert get_dotted_field_value_owned).
        assert_eq!(
            get_dotted_field_value(&event, &["a".into(), "0:2".into()]),
            None
        );
    }

    #[test]
    fn pop_missing_leaf_still_drops_empty_parents() {
        // helper.py/_pop_field_value_and_drop_empty entfernt leere
        // Eltern-Dicts auch, wenn der Leaf nicht existiert.
        let mut event = json!({"a": {"b": {}}, "keep": 1});
        let popped =
            pop_dotted_field_value(&mut event, &["a".into(), "b".into(), "c".into()], true);
        assert_eq!(popped, None);
        assert_eq!(event, json!({"keep": 1}));
    }
}
