//! key_checker in Rust (Phase 4i, Welle A).
//!
//! Checks whether all configured source fields exist in the event and collects
//! the missing ones (sorted, deduplicated) into the target field. The logic
//! mirrors `logprep/ng/processor/key_checker/processor.py`:
//!
//! ```python
//! not_existing_fields = list({f for f in source_fields if not _field_exists(event, f)})
//! if not_existing_fields:
//!     output_value = get_dotted_field_value(event, target_field)
//!     if isinstance(output_value, Iterable):
//!         output_value = list({*not_existing_fields, *output_value})
//!     else:
//!         output_value = not_existing_fields
//!     self._write_target_field(event, rule, sorted(output_value))
//! ```

use std::collections::BTreeSet;

use pyo3::prelude::*;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::field::value::{
    add_field_to, get_dotted_field_list, get_dotted_field_value, has_dotted_field, FieldExistsError,
};
use crate::filter::expression::pydict_to_json;

use super::{PyProcessorCore, PyRuleSpec, RuleSpec, SpecError, SpecWarning};
use super::spec_helper::validate_required_keys;

/// Python-`str()`-Darstellung eines JSON-Werts (fuer das Set-Mergen, das in
/// Python die rohen Objekte ueber das Unpacking aufnimmt; die Sortierung der
/// Set-Elemente betrifft realistisch nur Strings).
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

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyCheckerRuleSpec {
    source_fields: Vec<String>,
    target_field: String,
    #[serde(default)]
    overwrite_target: bool,
    #[serde(default)]
    merge_with_target: bool,
}

impl RuleSpec for KeyCheckerRuleSpec {
    fn type_name(&self) -> &'static str {
        "key_checker"
    }

    fn validate(raw: &Map<String, Value>) -> Result<(), String> {
        validate_required_keys(raw, &["source_fields", "target_field"])
    }

    fn apply(&self, event: &mut Value, warnings: &mut Vec<SpecWarning>) -> Result<(), SpecError> {
        let missing: BTreeSet<String> = self
            .source_fields
            .iter()
            .filter(|field| {
                let parts = get_dotted_field_list(field);
                !has_dotted_field(event, &parts, true)
            })
            .cloned()
            .collect();

        if missing.is_empty() {
            return Ok(());
        }

        // `isinstance(output_value, Iterable)` gilt in Python fuer
        // Listen (Elemente), Strings (Zeichen) und Dicts (Keys); alle
        // anderen Werte (None, Zahlen, ...) werden verworfen.
        let target_parts = get_dotted_field_list(&self.target_field);
        let merged: BTreeSet<String> = match get_dotted_field_value(event, &target_parts) {
            Some(Value::Array(items)) => {
                let mut set = missing;
                for item in items {
                    set.insert(value_to_python_str(item));
                }
                set
            }
            Some(Value::String(s)) => {
                let mut set = missing;
                set.extend(s.chars().map(String::from));
                set
            }
            Some(Value::Object(map)) => {
                let mut set = missing;
                set.extend(map.keys().cloned());
                set
            }
            _ => missing,
        };

        let content = Value::Array(merged.into_iter().map(Value::String).collect());
        match add_field_to(
            event,
            &target_parts,
            &content,
            self.merge_with_target,
            self.overwrite_target,
        ) {
            Ok(()) => Ok(()),
            Err(FieldExistsError { skipped_fields }) => {
                warnings.push(SpecWarning::FieldExists { skipped_fields });
                Ok(())
            }
        }
    }
}

/// Registriert einen `KeyCheckerRuleSpec` am `PyProcessorCore`.
///
/// Wird vom ng-Adapter-Konstruktor einmal erzeugt und pro `add_rule`
/// (i.d.R. waehrend `load_rules`) mit den Rule-Config-Werten gerufen.
#[pyclass(name = "PyKeyCheckerSpecFactory")]
pub struct PyKeyCheckerSpecFactory;

#[pymethods]
impl PyKeyCheckerSpecFactory {
    #[new]
    fn new() -> Self {
        Self
    }

    #[pyo3(signature = (core, rule_id, rule_data))]
    fn make_and_register(
        &self,
        py: Python<'_>,
        core: &mut PyProcessorCore,
        rule_id: u64,
        rule_data: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let Value::Object(raw) = pydict_to_json(rule_data)? else {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "rule_data must be a dictionary",
            ));
        };
        KeyCheckerRuleSpec::validate(&raw).map_err(pyo3::exceptions::PyValueError::new_err)?;
        let spec: KeyCheckerRuleSpec = serde_json::from_value(Value::Object(raw))
            .map_err(|err| pyo3::exceptions::PyValueError::new_err(err.to_string()))?;
        core.set_rule_spec(py, rule_id, Py::new(py, PyRuleSpec::new(Box::new(spec)))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::expression::{FilterExpressionInner, PyFilterExpression};
    use crate::rule::PyRuleTree;
    use pyo3::types::PyDict;
    use serde_json::json;

    fn spec(source_fields: Vec<&str>) -> KeyCheckerRuleSpec {
        KeyCheckerRuleSpec {
            source_fields: source_fields.into_iter().map(String::from).collect(),
            target_field: "missing_fields".into(),
            overwrite_target: false,
            merge_with_target: false,
        }
    }

    fn apply(
        spec: &KeyCheckerRuleSpec,
        event: &mut Value,
    ) -> (Result<(), SpecError>, Vec<SpecWarning>) {
        let mut warnings = Vec::new();
        let result = spec.apply(event, &mut warnings);
        (result, warnings)
    }

    #[test]
    fn validate_requires_source_fields_and_target_field() {
        let raw: Map<String, Value> = Map::new();
        assert!(KeyCheckerRuleSpec::validate(&raw).is_err());
        let raw: Map<String, Value> = serde_json::from_value(json!({"target_field": "t"}))
            .unwrap();
        assert!(KeyCheckerRuleSpec::validate(&raw).is_err());
        let raw: Map<String, Value> = serde_json::from_value(json!({
            "source_fields": ["a"],
            "target_field": "t",
        }))
        .unwrap();
        assert!(KeyCheckerRuleSpec::validate(&raw).is_ok());
    }

    #[test]
    fn rejects_unknown_config_keys() {
        let rule_data = json!({
            "source_fields": ["a"],
            "target_field": "t",
            "bogus": true,
        });
        let result: Result<KeyCheckerRuleSpec, _> = serde_json::from_value(rule_data);
        assert!(result.is_err());
    }

    #[test]
    fn writes_missing_root_key() {
        let mut event = json!({"testkey": "key1_value", "_index": "value"});
        let (result, warnings) = apply(&spec(vec!["key2"]), &mut event);
        assert!(result.is_ok());
        assert!(warnings.is_empty());
        assert_eq!(
            event,
            json!({"testkey": "key1_value", "_index": "value", "missing_fields": ["key2"]})
        );
    }

    #[test]
    fn writes_missing_sub_key() {
        let mut event = json!({"testkey": {"key1": "key1_value", "_index": "value"}});
        let (result, _) = apply(&spec(vec!["testkey.key2"]), &mut event);
        assert!(result.is_ok());
        assert_eq!(event.get("missing_fields"), Some(&json!(["testkey.key2"])));
    }

    #[test]
    fn writes_only_missing_of_many() {
        let mut event = json!({"key1": {"key2": {"key3": {"key3": "v"}, "random_key": "r"}, "_index": "value"}});
        let (result, _) = apply(
            &spec(vec!["key1.key2", "key1", "key1.key2.key3", "key4"]),
            &mut event,
        );
        assert!(result.is_ok());
        assert_eq!(event.get("missing_fields"), Some(&json!(["key4"])));
    }

    #[test]
    fn all_keys_present_leaves_event_unchanged() {
        let mut event = json!({"key1": {"key2": "v"}});
        let (result, warnings) = apply(&spec(vec!["key1", "key1.key2"]), &mut event);
        assert!(result.is_ok());
        assert!(warnings.is_empty());
        assert_eq!(event, json!({"key1": {"key2": "v"}}));
    }

    #[test]
    fn duplicate_source_fields_dedup() {
        let mut event = json!({"randomkey": "x"});
        let (result, _) = apply(&spec(vec!["key1", "key1"]), &mut event);
        assert!(result.is_ok());
        assert_eq!(event.get("missing_fields"), Some(&json!(["key1"])));
    }

    #[test]
    fn result_is_sorted() {
        let mut event = json!({"b": "x"});
        let (result, _) = apply(&spec(vec!["c", "a"]), &mut event);
        assert!(result.is_ok());
        assert_eq!(event.get("missing_fields"), Some(&json!(["a", "c"])));
    }

    #[test]
    fn null_value_counts_as_existing() {
        // has_dotted_field allow_none=true wie der Python-Helper-Default
        let mut event = json!({"key1": null});
        let (result, warnings) = apply(&spec(vec!["key1"]), &mut event);
        assert!(result.is_ok());
        assert!(warnings.is_empty());
        assert!(!event.as_object().unwrap().contains_key("missing_fields"));
    }

    #[test]
    fn extends_existing_list_with_overwrite_target() {
        let mut rule_spec = spec(vec!["not.existing.key"]);
        rule_spec.overwrite_target = true;
        let mut event = json!({"missing_fields": ["i.exists.already"]});
        let (result, warnings) = apply(&rule_spec, &mut event);
        assert!(result.is_ok());
        assert!(warnings.is_empty());
        assert_eq!(
            event.get("missing_fields"),
            Some(&json!(["i.exists.already", "not.existing.key"]))
        );
    }

    #[test]
    fn prevents_duplicates_in_existing_list() {
        let mut rule_spec = spec(vec!["not.existing.key"]);
        rule_spec.overwrite_target = true;
        let mut event = json!({"missing_fields": ["not.existing.key"]});
        let (result, _) = apply(&rule_spec, &mut event);
        assert!(result.is_ok());
        assert_eq!(
            event.get("missing_fields"),
            Some(&json!(["not.existing.key"]))
        );
    }

    #[test]
    fn existing_target_produces_field_exists_warning() {
        let mut event = json!({"missing_fields": ["i.exists.already"]});
        let (result, warnings) = apply(&spec(vec!["not.existing.key"]), &mut event);
        assert!(result.is_ok());
        assert_eq!(warnings.len(), 1);
        match &warnings[0] {
            SpecWarning::FieldExists { skipped_fields } => {
                assert_eq!(skipped_fields, &vec!["missing_fields".to_string()]);
            }
            other => panic!("expected FieldExists warning, got {other:?}"),
        }
        // Target bleibt unveraendert
        assert_eq!(event.get("missing_fields"), Some(&json!(["i.exists.already"])));
    }

    #[test]
    fn escaped_dot_source_field() {
        // "dotted\.field" referenziert den Key "dotted.field"
        let mut event = json!({"dotted.field": "x"});
        let (result, warnings) = apply(&spec(vec![r"dotted\.field"]), &mut event);
        assert!(result.is_ok());
        assert!(warnings.is_empty());
        assert!(!event.as_object().unwrap().contains_key("missing_fields"));
    }

    #[test]
    fn string_target_expands_to_characters_with_overwrite() {
        // Python: isinstance(str, Iterable) → {*not_existing_fields, *"ab"},
        // geschrieben als Array (Overwrite erlaubt das Ersetzen des Targets).
        let mut rule_spec = spec(vec!["z"]);
        rule_spec.overwrite_target = true;
        let mut event = json!({"missing_fields": "ab"});
        let (result, warnings) = apply(&rule_spec, &mut event);
        assert!(result.is_ok());
        assert!(warnings.is_empty());
        assert_eq!(event.get("missing_fields"), Some(&json!(["a", "b", "z"])));
    }

    #[test]
    fn existing_string_target_without_overwrite_warns() {
        let mut event = json!({"missing_fields": "ab"});
        let (result, warnings) = apply(&spec(vec!["z"]), &mut event);
        assert!(result.is_ok());
        assert_eq!(warnings.len(), 1);
        match &warnings[0] {
            SpecWarning::FieldExists { skipped_fields } => {
                assert_eq!(skipped_fields, &vec!["missing_fields".to_string()]);
            }
            other => panic!("expected FieldExists warning, got {other:?}"),
        }
        assert_eq!(event.get("missing_fields"), Some(&json!("ab")));
    }

    // ─── Factory (braucht den Python-Interpreter) ─────────────────────────

    fn py_ready() {
        pyo3::prepare_freethreaded_python();
    }

    fn add_to_tree(
        py: Python<'_>,
        tree: &Py<PyRuleTree>,
        rule_id: u64,
        expr: FilterExpressionInner,
    ) -> PyResult<()> {
        use pyo3::types::PyList;
        let segments = PyList::empty(py);
        let segment = PyList::new(py, [Py::new(py, PyFilterExpression { inner: expr })?])?;
        segments.append(segment)?;
        tree.bind(py).borrow_mut().add_rule(rule_id, &segments)
    }

    fn make_fake_rule(py: Python<'_>) -> PyResult<PyObject> {
        let code = pyo3::ffi::c_str!(
            "type('FakeRule', (), {\n\
                'filter': None,\n\
                'data_error': None,\n\
                'failure_tags': [],\n\
                'delete_source_fields': False,\n\
                'source_fields': [],\n\
                'id': 'kc-fake',\n\
                'description': 'fake rule',\n\
            })"
        );
        py.eval(code, None, None).map(|t| t.unbind())
    }

    #[test]
    fn factory_requires_dict_rule_data() {
        py_ready();
        Python::with_gil(|py| {
            let factory = PyKeyCheckerSpecFactory::new();
            let mut core = PyProcessorCore::new(false, false);
            let list = pyo3::types::PyList::empty(py);
            let err = factory
                .make_and_register(py, &mut core, 1, list.as_any())
                .unwrap_err();
            assert!(err.to_string().contains("dict"));
        });
    }

    #[test]
    fn factory_missing_required_keys_raises_value_error() {
        py_ready();
        Python::with_gil(|py| {
            let factory = PyKeyCheckerSpecFactory::new();
            let mut core = PyProcessorCore::new(false, false);
            let rule_data = PyDict::new(py);
            rule_data.set_item("target_field", "t").unwrap();
            let err = factory
                .make_and_register(py, &mut core, 1, rule_data.as_any())
                .unwrap_err();
            assert!(err.to_string().contains("missing required key"));
        });
    }

    #[test]
    fn factory_unknown_config_key_raises_value_error() {
        py_ready();
        Python::with_gil(|py| {
            let factory = PyKeyCheckerSpecFactory::new();
            let mut core = PyProcessorCore::new(false, false);
            let rule_data = PyDict::new(py);
            let source_fields = pyo3::types::PyList::new(py, ["a"]).unwrap();
            rule_data.set_item("source_fields", source_fields).unwrap();
            rule_data.set_item("target_field", "t").unwrap();
            rule_data.set_item("bogus", true).unwrap();
            let err = factory
                .make_and_register(py, &mut core, 1, rule_data.as_any())
                .unwrap_err();
            assert!(err.to_string().contains("bogus"));
        });
    }

    #[test]
    fn factory_registered_spec_applies_on_matched_rule() {
        py_ready();
        Python::with_gil(|py| {
            let factory = PyKeyCheckerSpecFactory::new();
            let mut core = PyProcessorCore::new(false, false);

            // Baum mit einer Regel (key "f" == "v") + Rule-Mapping
            let tree = Py::new(py, PyRuleTree::new()).unwrap();
            add_to_tree(
                py,
                &tree,
                7,
                FilterExpressionInner::String {
                    key: vec!["f".into()],
                    expected: "v".into(),
                },
            )
            .unwrap();
            let rule = make_fake_rule(py).unwrap();
            let mapping = PyDict::new(py);
            mapping.set_item(7u64, &rule).unwrap();
            core.set_tree(tree, mapping.unbind());

            // Spec registrieren
            let rule_data = PyDict::new(py);
            let source_fields = pyo3::types::PyList::new(py, ["key2"]).unwrap();
            rule_data.set_item("source_fields", &source_fields).unwrap();
            rule_data.set_item("target_field", "missing_fields").unwrap();
            factory
                .make_and_register(py, &mut core, 7, rule_data.as_any())
                .unwrap();

            // Event matcht die Regel → Spec muss die fehlenden Felder schreiben
            let event = PyDict::new(py);
            event.set_item("f", "v").unwrap();
            event.set_item("key1", "value").unwrap();
            let outcome = core.process(py, &event, None).unwrap();
            assert_eq!(outcome.matched_rule_ids, vec![7]);
            assert!(outcome.warnings.is_empty());
            assert!(outcome.errors.is_empty());
            let written: Vec<String> = event
                .get_item("missing_fields")
                .unwrap()
                .unwrap()
                .extract()
                .unwrap();
            assert_eq!(written, vec!["key2".to_string()]);
        });
    }
}