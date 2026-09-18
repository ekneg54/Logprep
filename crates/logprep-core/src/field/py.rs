//! PyO3 wrappers for the legacy Python API (`logprep.util.helper`).
//!
//! Thin adapters (Phase 4a): every function converts the event dict to
//! `serde_json::Value`, calls the pure-Rust implementation in `super::value`
//! and writes mutations back into the live Python dict. The pure logic lives
//! in `field::value` — this module must not contain processing logic.

use pyo3::prelude::*;
use pyo3::sync::GILOnceCell;
use pyo3::types::PyDict;

use crate::filter::expression::{json_to_pyany, json_to_pydict, pyany_to_json, pydict_to_json};

use super::value;

static MISSING_CELL: GILOnceCell<PyObject> = GILOnceCell::new();
static SKIP_CELL: GILOnceCell<PyObject> = GILOnceCell::new();

fn get_missing(py: Python) -> PyResult<PyObject> {
    Ok(MISSING_CELL
        .get_or_try_init(py, || {
            let helper = py.import("logprep.util.helper")?;
            Ok(helper.getattr("MISSING")?.unbind()) as PyResult<PyObject>
        })?
        .clone_ref(py))
}

fn get_skip(py: Python) -> PyResult<PyObject> {
    Ok(SKIP_CELL
        .get_or_try_init(py, || {
            let helper = py.import("logprep.util.helper")?;
            Ok(helper.getattr("SKIP")?.unbind()) as PyResult<PyObject>
        })?
        .clone_ref(py))
}

fn raise_field_exists_warning(
    py: Python,
    rule: Option<&Bound<'_, pyo3::PyAny>>,
    event: &Bound<'_, pyo3::PyAny>,
    skipped_fields: Vec<String>,
) -> PyResult<PyErr> {
    let exceptions = py.import("logprep.processor.base.exceptions")?;
    let cls = exceptions.getattr("FieldExistsWarning")?;
    let rule_py: PyObject = match rule {
        Some(r) => r.clone().unbind(),
        None => py.None(),
    };
    let event_py: PyObject = event.clone().unbind();
    let skipped_py: PyObject = skipped_fields.into_pyobject(py)?.into_any().unbind();
    let exc = cls.call1((rule_py, event_py, skipped_py))?;
    Ok(PyErr::from_value(exc))
}

/// Converts the live event dict to a JSON value; `None` for non-dict events
/// (which mirror the old "container is neither dict nor list" miss paths).
fn event_to_json(event: &Bound<'_, pyo3::PyAny>) -> Option<serde_json::Value> {
    pydict_to_json(event).ok()
}

// =============================================================================
// Parsing (Step 1c)
// =============================================================================

/// Splits a dotted field string into its components.
/// Supports escaping: "dotted\.field" -> ["dotted.field"]
#[pyfunction]
pub fn get_dotted_field_list(dotted_field: &str) -> Vec<String> {
    value::get_dotted_field_list(dotted_field)
}

/// Combines a field list into a dotted field.
/// Dots in field names are escaped: ["x.y", "z"] -> "x\\.y.z"
#[pyfunction]
pub fn field_list_to_dotted_field(field_list: &Bound<'_, pyo3::PyAny>) -> PyResult<String> {
    let fields: Vec<String> = field_list
        .try_iter()?
        .map(|item| item.and_then(|i| i.extract::<String>()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(value::field_list_to_dotted_field(&fields))
}

/// Combines dotted fields without escaping: ["x.y", "z"] -> "x.y.z"
#[pyfunction]
pub fn join_dotted_fields(dotted_fields: Vec<String>) -> String {
    value::join_dotted_fields(&dotted_fields)
}

// =============================================================================
// Read (Step 1d)
// =============================================================================

/// Returns the value of a dotted field from an event dict, or None if not found.
#[pyfunction]
pub fn get_dotted_field_value(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    dotted_field: &str,
) -> PyResult<PyObject> {
    let parts = value::get_dotted_field_list(dotted_field);
    let found =
        event_to_json(event).and_then(|doc| value::get_dotted_field_value_owned(&doc, &parts));
    match found {
        Some(v) => json_to_pyany(py, &v),
        None => Ok(py.None()),
    }
}

/// Returns the value of a dotted field, or MISSING if not found.
#[pyfunction]
pub fn get_dotted_field_value_with_missing(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    dotted_field: &str,
) -> PyResult<PyObject> {
    let missing = get_missing(py)?;
    let parts = value::get_dotted_field_list(dotted_field);
    let found =
        event_to_json(event).and_then(|doc| value::get_dotted_field_value_owned(&doc, &parts));
    match found {
        Some(v) => json_to_pyany(py, &v),
        None => Ok(missing),
    }
}

/// Returns the value referenced by a sequence of field keys, or MISSING if not found.
#[pyfunction]
pub fn get_field_value(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    fields: Vec<String>,
) -> PyResult<PyObject> {
    let missing = get_missing(py)?;
    let found =
        event_to_json(event).and_then(|doc| value::get_dotted_field_value_owned(&doc, &fields));
    match found {
        Some(v) => json_to_pyany(py, &v),
        None => Ok(missing),
    }
}

/// Optimized: returns value by field sequence without slice support, or MISSING if not found.
#[pyfunction]
pub fn get_field_value_no_slice(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    fields: Vec<String>,
) -> PyResult<PyObject> {
    let missing = get_missing(py)?;
    let Some(doc) = event_to_json(event) else {
        return Ok(missing);
    };
    let mut current = &doc;
    for field in &fields {
        let Some(next) = current.as_object().and_then(|map| map.get(field.as_str())) else {
            return Ok(missing);
        };
        current = next;
    }
    json_to_pyany(py, current)
}

/// Batch extraction of dotted fields from an event.
#[pyfunction]
#[pyo3(signature = (event, dotted_fields, on_missing=None))]
pub fn get_dotted_field_values(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    dotted_fields: &Bound<'_, pyo3::PyAny>,
    on_missing: Option<PyObject>,
) -> PyResult<PyObject> {
    let skip = get_skip(py)?;
    let doc = event_to_json(event);
    let result = PyDict::new(py);
    let field_names: Vec<String> = dotted_fields
        .call_method0("__iter__")?
        .try_iter()?
        .map(|item| item.and_then(|i| i.extract::<String>()))
        .collect::<Result<Vec<_>, _>>()?;
    for field_name in &field_names {
        let parts = value::get_dotted_field_list(field_name);
        let found = doc
            .as_ref()
            .and_then(|event| value::get_dotted_field_value_owned(event, &parts));
        match found {
            Some(v) => result.set_item(field_name, json_to_pyany(py, &v)?)?,
            None => {
                if let Some(callback) = &on_missing {
                    let fallback = callback.call1(py, (field_name,))?;
                    if !fallback.bind(py).eq(skip.bind(py))? {
                        result.set_item(field_name, fallback)?;
                    }
                } else {
                    result.set_item(field_name, py.None())?;
                }
            }
        }
    }
    Ok(result.into_any().unbind())
}

// =============================================================================
// Existence (Step 1e)
// =============================================================================

/// Checks if a dotted field exists in an event.
#[pyfunction]
#[pyo3(signature = (event, dotted_field, allow_none=true))]
pub fn has_dotted_field(
    _py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    dotted_field: &str,
    allow_none: bool,
) -> PyResult<bool> {
    let parts = value::get_dotted_field_list(dotted_field);
    let found =
        event_to_json(event).and_then(|doc| value::get_dotted_field_value_owned(&doc, &parts));
    Ok(found.map(|v| allow_none || !v.is_null()).unwrap_or(false))
}

// =============================================================================
// Pop (Step 1f)
// =============================================================================

/// Removes and returns a dotted field value from an event.
#[pyfunction]
#[pyo3(signature = (event, dotted_field, drop_empty=true))]
pub fn pop_dotted_field_value(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    dotted_field: &str,
    drop_empty: bool,
) -> PyResult<PyObject> {
    let missing = get_missing(py)?;
    let Ok(dict) = event.downcast::<PyDict>() else {
        return Ok(missing);
    };
    let mut doc = pydict_to_json(dict.as_any())?;
    let parts = value::get_dotted_field_list(dotted_field);
    let popped = value::pop_dotted_field_value(&mut doc, &parts, drop_empty);
    // Ruecksynchronisieren — auch wenn nichts gefunden wurde, kann
    // drop_empty leere Eltern-Dicts entfernt haben.
    json_to_pydict(dict, &doc)?;
    match popped {
        Some(v) => json_to_pyany(py, &v),
        None => Ok(missing),
    }
}

// =============================================================================
// Write (Step 1g)
// =============================================================================

/// Adds fields to an event dict with proper error handling.
#[pyfunction]
#[pyo3(signature = (event, fields, rule=None, merge_with_target=false, overwrite_target=false, skip_none=true))]
pub fn add_fields_to(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    fields: &Bound<'_, PyDict>,
    rule: Option<&Bound<'_, pyo3::PyAny>>,
    merge_with_target: bool,
    overwrite_target: bool,
    skip_none: bool,
) -> PyResult<()> {
    if merge_with_target && overwrite_target {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "Can't merge with and overwrite a target field at the same time",
        ));
    }
    let dict = event.downcast::<PyDict>()?;
    let mut doc = pydict_to_json(dict.as_any())?;
    let mut items: Vec<(String, serde_json::Value)> = Vec::new();
    for (key, value_obj) in fields.iter() {
        let key = key.extract::<String>().unwrap_or_else(|_| key.to_string());
        items.push((key, pyany_to_json(&value_obj)?));
    }
    let outcome = value::add_fields_to(
        &mut doc,
        &items,
        merge_with_target,
        overwrite_target,
        skip_none,
    );
    json_to_pydict(dict, &doc)?;
    match outcome {
        Ok(()) => Ok(()),
        Err(err) => Err(raise_field_exists_warning(
            py,
            rule,
            event,
            err.skipped_fields,
        )?),
    }
}

// =============================================================================
// Tests (Step 1c)
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_dotted_field() {
        assert_eq!(get_dotted_field_list("a.b.c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn escaped_dot() {
        assert_eq!(
            get_dotted_field_list(r"dotted\.field"),
            vec!["dotted.field"]
        );
    }

    #[test]
    fn double_escaped() {
        assert_eq!(
            get_dotted_field_list(r"dotted\\.field"),
            vec![r"dotted\", "field"]
        );
    }

    #[test]
    fn no_dots() {
        assert_eq!(get_dotted_field_list("simple"), vec!["simple"]);
    }

    #[test]
    fn trailing_backslash() {
        assert_eq!(get_dotted_field_list(r"field\\"), vec![r"field\"]);
    }

    #[test]
    fn field_list_to_dotted_field_simple() {
        assert_eq!(
            crate::field::value::field_list_to_dotted_field(&["a".into(), "b".into(), "c".into()]),
            "a.b.c"
        );
    }

    #[test]
    fn field_list_to_dotted_field_escape() {
        assert_eq!(
            crate::field::value::field_list_to_dotted_field(&["x.y".into(), "z".into()]),
            "x\\.y.z"
        );
    }

    #[test]
    fn join_dotted_fields_simple() {
        assert_eq!(join_dotted_fields(vec!["x.y".into(), "z".into()]), "x.y.z");
    }
}
