use pyo3::prelude::*;
use pyo3::sync::GILOnceCell;
use pyo3::types::{PyDict, PyList};

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

// =============================================================================
// Parsing (Step 1c)
// =============================================================================

/// Splits a dotted field string into its components.
/// Supports escaping: "dotted\.field" -> ["dotted.field"]
#[pyfunction]
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

/// Internal pure-Rust implementation for combining field names with dot-escaping.
fn field_list_to_dotted_field_inner(field_list: &[String]) -> String {
    field_list
        .iter()
        .map(|field| field.replace(".", "\\."))
        .collect::<Vec<_>>()
        .join(".")
}

/// Combines a field list into a dotted field.
/// Dots in field names are escaped: ["x.y", "z"] -> "x\\.y.z"
#[pyfunction]
pub fn field_list_to_dotted_field(field_list: &Bound<'_, pyo3::PyAny>) -> PyResult<String> {
    let fields: Vec<String> = field_list
        .try_iter()?
        .map(|item| item.and_then(|i| i.extract::<String>()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(field_list_to_dotted_field_inner(&fields))
}

/// Combines dotted fields without escaping: ["x.y", "z"] -> "x.y.z"
#[pyfunction]
pub fn join_dotted_fields(dotted_fields: Vec<String>) -> String {
    dotted_fields.join(".")
}

// =============================================================================
// Low-Level (Step 1d)
// =============================================================================

fn get_slice_arg(slice_item: &str) -> PyResult<Option<isize>> {
    if slice_item.is_empty() {
        return Ok(None);
    }
    slice_item.parse::<isize>().map(Some).map_err(|_| {
        pyo3::exceptions::PyValueError::new_err(format!("Invalid slice arg: {}", slice_item))
    })
}

fn get_item<'py>(
    _py: Python<'py>,
    container: &Bound<'py, pyo3::PyAny>,
    key: &str,
) -> PyResult<Bound<'py, pyo3::PyAny>> {
    if let Ok(dict) = container.downcast::<PyDict>() {
        return dict
            .get_item(key)?
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(key.to_string()));
    }

    if let Ok(_list) = container.downcast::<PyList>() {
        if key.contains(':') {
            let parts: Vec<&str> = key.split(':').collect();
            let start = get_slice_arg(parts.first().copied().unwrap_or(""))?;
            let stop = get_slice_arg(parts.get(1).copied().unwrap_or(""))?;
            let step = get_slice_arg(parts.get(2).copied().unwrap_or(""))?.unwrap_or(1);
            let py = container.py();
            let builtins = py.import("builtins")?;
            let slice_fn = builtins.getattr("slice")?;
            let start_obj: PyObject = match start {
                Some(v) => v.into_pyobject(py)?.into_any().unbind(),
                None => py.None(),
            };
            let stop_obj: PyObject = match stop {
                Some(v) => v.into_pyobject(py)?.into_any().unbind(),
                None => py.None(),
            };
            let step_obj: PyObject = step.into_pyobject(py)?.into_any().unbind();
            let slice = slice_fn.call1((start_obj, stop_obj, step_obj))?;
            return container.get_item(slice);
        }
        let index: isize = key.parse().map_err(|_| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid index: {}", key))
        })?;
        let len = container.len()?;
        let abs_index = if index < 0 {
            (len as isize + index).try_into().map_err(|_| {
                pyo3::exceptions::PyValueError::new_err(format!("Index out of range: {}", index))
            })?
        } else {
            index as usize
        };
        return container.get_item(abs_index);
    }

    Err(pyo3::exceptions::PyTypeError::new_err(
        "Container is neither dict nor list",
    ))
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
    let parts = get_dotted_field_list(dotted_field);
    let mut current = event.clone();
    for part in &parts {
        current = match get_item(py, &current, part) {
            Ok(val) => val,
            Err(_) => return Ok(py.None()),
        };
    }
    Ok(current.unbind())
}

/// Returns the value of a dotted field, or MISSING if not found.
#[pyfunction]
pub fn get_dotted_field_value_with_missing(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    dotted_field: &str,
) -> PyResult<PyObject> {
    let missing = get_missing(py)?;
    let parts = get_dotted_field_list(dotted_field);
    let mut current = event.clone();
    for part in &parts {
        current = match get_item(py, &current, part) {
            Ok(val) => val,
            Err(_) => return Ok(missing.clone_ref(py)),
        };
    }
    Ok(current.unbind())
}

/// Returns the value referenced by a sequence of field keys, or MISSING if not found.
#[pyfunction]
pub fn get_field_value(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    fields: Vec<String>,
) -> PyResult<PyObject> {
    let missing = get_missing(py)?;
    let mut current = event.clone();
    for field in &fields {
        current = match get_item(py, &current, field) {
            Ok(val) => val,
            Err(_) => return Ok(missing.clone_ref(py)),
        };
    }
    Ok(current.unbind())
}

/// Optimized: returns value by field sequence without slice support, or MISSING if not found.
#[pyfunction]
pub fn get_field_value_no_slice(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    fields: Vec<String>,
) -> PyResult<PyObject> {
    let missing = get_missing(py)?;
    let mut current = event.clone();
    for field in &fields {
        if let Ok(dict) = current.downcast::<PyDict>() {
            current = match dict.get_item(field)? {
                Some(val) => val,
                None => return Ok(missing.clone_ref(py)),
            };
        } else {
            return Ok(missing.clone_ref(py));
        }
    }
    Ok(current.unbind())
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

    let field_names: Vec<String> = dotted_fields
        .call_method0("__iter__")?
        .try_iter()?
        .map(|item| item.and_then(|i| i.extract::<String>()))
        .collect::<Result<Vec<_>, _>>()?;

    let result = PyDict::new(py);
    for field_name in &field_names {
        let parts = get_dotted_field_list(field_name);
        let mut current = event.clone();
        let mut found = true;
        for part in &parts {
            match get_item(py, &current, part) {
                Ok(val) => current = val,
                Err(_) => {
                    found = false;
                    break;
                }
            }
        }

        if found {
            result.set_item(field_name, current.unbind())?;
        } else if let Some(ref callback) = on_missing {
            let fallback = callback.call1(py, (field_name,))?;
            if !fallback.bind(py).eq(skip.bind(py))? {
                result.set_item(field_name, fallback)?;
            }
        } else {
            result.set_item(field_name, py.None())?;
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
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    dotted_field: &str,
    allow_none: bool,
) -> PyResult<bool> {
    let parts = get_dotted_field_list(dotted_field);
    let mut current = event.clone();

    for part in &parts {
        current = match get_item(py, &current, part) {
            Ok(val) => val,
            Err(_) => return Ok(false),
        };
    }

    if allow_none {
        Ok(true)
    } else {
        Ok(!current.is_none())
    }
}

// =============================================================================
// Pop (Step 1f)
// =============================================================================

fn pop_field_value<'py>(
    py: Python<'py>,
    event: &Bound<'py, pyo3::PyAny>,
    dotted_field: &str,
) -> PyResult<PyObject> {
    let missing = get_missing(py)?;
    let parts = get_dotted_field_list(dotted_field);
    if parts.is_empty() {
        return Ok(missing.clone_ref(py));
    }

    let parent_fields = &parts[..parts.len() - 1];
    let last_key = &parts[parts.len() - 1];

    let parent = if parent_fields.is_empty() {
        event.clone()
    } else {
        let mut current = event.clone();
        for part in parent_fields {
            current = get_item(py, &current, part)?;
        }
        current
    };

    if let Ok(dict) = parent.downcast::<PyDict>() {
        match dict.get_item(last_key)? {
            Some(value) => {
                dict.del_item(last_key)?;
                Ok(value.unbind())
            }
            None => Ok(missing.clone_ref(py)),
        }
    } else {
        Ok(missing.clone_ref(py))
    }
}

fn pop_field_value_and_drop_empty<'py>(
    py: Python<'py>,
    event: &Bound<'py, pyo3::PyAny>,
    parts: &[String],
) -> PyResult<PyObject> {
    let missing = get_missing(py)?;

    if parts.is_empty() {
        return Ok(missing.clone_ref(py));
    }

    let next_key = &parts[0];
    let remaining = &parts[1..];

    if let Ok(dict) = event.downcast::<PyDict>() {
        match dict.get_item(next_key)? {
            Some(child) => {
                if remaining.is_empty() {
                    let value = child.unbind();
                    dict.del_item(next_key)?;
                    return Ok(value);
                }

                let value = pop_field_value_and_drop_empty(py, &child, remaining)?;

                if let Ok(child_dict) = child.downcast::<PyDict>()
                    && child_dict.is_empty()
                {
                    dict.del_item(next_key)?;
                }

                Ok(value)
            }
            None => Ok(missing.clone_ref(py)),
        }
    } else {
        Ok(missing.clone_ref(py))
    }
}

/// Removes and returns a dotted field value from an event.
#[pyfunction]
#[pyo3(signature = (event, dotted_field, drop_empty=true))]
pub fn pop_dotted_field_value(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    dotted_field: &str,
    drop_empty: bool,
) -> PyResult<PyObject> {
    let parts = get_dotted_field_list(dotted_field);

    if drop_empty {
        pop_field_value_and_drop_empty(py, event, &parts)
    } else {
        pop_field_value(py, event, dotted_field)
    }
}

// =============================================================================
// Write (Step 1g)
// =============================================================================

fn add_and_overwrite_key<'py>(
    py: Python<'py>,
    event: &Bound<'py, pyo3::PyAny>,
    key: &str,
) -> PyResult<Bound<'py, pyo3::PyAny>> {
    if let Ok(dict) = event.downcast::<PyDict>() {
        if let Ok(Some(existing)) = dict.get_item(key)
            && existing.downcast::<PyDict>().is_ok()
        {
            return Ok(existing);
        }
        let sub_dict = PyDict::new(py);
        dict.set_item(key, &sub_dict)?;
        return Ok(sub_dict.into_any());
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "Container is not a dict",
    ))
}

fn add_and_not_overwrite_key<'py>(
    py: Python<'py>,
    event: &Bound<'py, pyo3::PyAny>,
    key: &str,
) -> PyResult<Bound<'py, pyo3::PyAny>> {
    if let Ok(dict) = event.downcast::<PyDict>() {
        if let Ok(Some(existing)) = dict.get_item(key) {
            if existing.downcast::<PyDict>().is_ok() {
                return Ok(existing);
            }
            return Err(pyo3::exceptions::PyKeyError::new_err("key exists"));
        }
        let sub_dict = PyDict::new(py);
        dict.set_item(key, &sub_dict)?;
        return Ok(sub_dict.into_any());
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "Container is not a dict",
    ))
}

fn add_field_to(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    field_name: &str,
    content: &Bound<'_, pyo3::PyAny>,
    rule: Option<&Bound<'_, pyo3::PyAny>>,
    merge_with_target: bool,
    overwrite_target: bool,
) -> PyResult<()> {
    if merge_with_target && overwrite_target {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "Can't merge with and overwrite a target field at the same time",
        ));
    }

    let parts = get_dotted_field_list(field_name);
    if parts.is_empty() {
        return Err(pyo3::exceptions::PyValueError::new_err("Empty field path"));
    }
    let target_key = &parts[parts.len() - 1];

    if overwrite_target {
        let mut current = event.clone();
        for part in &parts[..parts.len() - 1] {
            current = add_and_overwrite_key(py, &current, part)?;
        }
        if let Ok(dict) = current.downcast::<PyDict>() {
            let copy_mod = py.import("copy")?;
            let deep_copy = copy_mod.getattr("deepcopy")?;
            let copied = deep_copy.call1((content,))?;
            dict.set_item(target_key, copied)?;
        }
        return Ok(());
    }

    let mut current = event.clone();
    for part in &parts[..parts.len() - 1] {
        current = match add_and_not_overwrite_key(py, &current, part) {
            Ok(val) => val,
            Err(_) => {
                return Err(raise_field_exists_warning(
                    py,
                    rule,
                    event,
                    vec![field_name.to_string()],
                )?);
            }
        };
    }

    if let Ok(dict) = current.downcast::<PyDict>() {
        let existing = dict.get_item(target_key)?;

        match existing {
            Some(existing_val) if existing_val.is_none() => {
                let copy_mod = py.import("copy")?;
                let deep_copy = copy_mod.getattr("deepcopy")?;
                dict.set_item(target_key, deep_copy.call1((content,))?)?;
            }
            None => {
                let copy_mod = py.import("copy")?;
                let deep_copy = copy_mod.getattr("deepcopy")?;
                dict.set_item(target_key, deep_copy.call1((content,))?)?;
            }
            Some(existing_val) => {
                if !merge_with_target {
                    return Err(raise_field_exists_warning(
                        py,
                        rule,
                        event,
                        vec![field_name.to_string()],
                    )?);
                }

                if let (Ok(existing_dict), Ok(content_dict)) = (
                    existing_val.downcast::<PyDict>(),
                    content.downcast::<PyDict>(),
                ) {
                    existing_dict.update(content_dict.as_mapping())?;
                } else if let (Ok(existing_list), Ok(content_list)) = (
                    existing_val.downcast::<PyList>(),
                    content.downcast::<PyList>(),
                ) {
                    for item in content_list {
                        existing_list.append(item)?;
                    }
                } else if let Ok(existing_list) = existing_val.downcast::<PyList>() {
                    existing_list.append(content)?;
                } else if let Ok(content_list) = content.downcast::<PyList>() {
                    let new_list = PyList::empty(py);
                    new_list.append(existing_val)?;
                    for item in content_list {
                        new_list.append(item)?;
                    }
                    dict.set_item(target_key, new_list)?;
                } else {
                    if !overwrite_target {
                        return Err(raise_field_exists_warning(
                            py,
                            rule,
                            event,
                            vec![field_name.to_string()],
                        )?);
                    }
                    let new_list = PyList::empty(py);
                    new_list.append(existing_val)?;
                    let copy_mod = py.import("copy")?;
                    let deep_copy = copy_mod.getattr("deepcopy")?;
                    new_list.append(deep_copy.call1((content,))?)?;
                    dict.set_item(target_key, new_list)?;
                }
            }
        }
    }

    Ok(())
}

fn add_field_to_silent_fail(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    field_name: &str,
    content: &Bound<'_, pyo3::PyAny>,
    rule: Option<&Bound<'_, pyo3::PyAny>>,
    merge_with_target: bool,
    overwrite_target: bool,
) -> PyResult<Option<String>> {
    match add_field_to(
        py,
        event,
        field_name,
        content,
        rule,
        merge_with_target,
        overwrite_target,
    ) {
        Ok(()) => Ok(None),
        Err(e) => {
            let exceptions = py.import("logprep.processor.base.exceptions")?;
            let cls = exceptions.getattr("FieldExistsWarning")?;
            if e.is_instance(py, &cls) {
                let skipped = e.value(py).getattr("skipped_fields")?;
                let first = skipped.get_item(0)?;
                Ok(Some(first.to_string()))
            } else {
                Err(e)
            }
        }
    }
}

/// Adds fields to an event dict with proper error handling.
#[pyfunction]
#[pyo3(signature = (event, fields, rule=None, merge_with_target=false, overwrite_target=false, skip_none=true))]
pub fn add_fields_to(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    fields: &Bound<'_, pyo3::types::PyDict>,
    rule: Option<&Bound<'_, pyo3::PyAny>>,
    merge_with_target: bool,
    overwrite_target: bool,
    skip_none: bool,
) -> PyResult<()> {
    let filtered: Vec<(String, PyObject)> = fields
        .iter()
        .filter(|(_, v)| !skip_none || !v.is_none())
        .map(|(k, v)| (k.to_string(), v.unbind()))
        .collect();

    let num_fields = filtered.len();

    if num_fields == 1 {
        let (field_name, value) = &filtered[0];
        let value_bound = value.bind(py);
        add_field_to(
            py,
            event,
            field_name,
            value_bound,
            rule,
            merge_with_target,
            overwrite_target,
        )?;
        return Ok(());
    }

    let mut unsuccessful = Vec::new();
    for (field_name, value) in &filtered {
        let value_bound = value.bind(py);
        if let Ok(Some(skipped)) = add_field_to_silent_fail(
            py,
            event,
            field_name,
            value_bound,
            rule,
            merge_with_target,
            overwrite_target,
        ) {
            unsuccessful.push(skipped);
        }
    }

    if !unsuccessful.is_empty() {
        return Err(raise_field_exists_warning(py, rule, event, unsuccessful)?);
    }

    Ok(())
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
            field_list_to_dotted_field_inner(&["a".into(), "b".into(), "c".into()]),
            "a.b.c"
        );
    }

    #[test]
    fn field_list_to_dotted_field_escape() {
        assert_eq!(
            field_list_to_dotted_field_inner(&["x.y".into(), "z".into()]),
            "x\\.y.z"
        );
    }

    #[test]
    fn join_dotted_fields_simple() {
        assert_eq!(join_dotted_fields(vec!["x.y".into(), "z".into()]), "x.y.z");
    }
}

