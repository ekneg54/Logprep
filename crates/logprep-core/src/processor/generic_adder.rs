//! generic_adder in Rust (Phase 4i, Welle A).
//!
//! Der Rust-Teil schreibt ausschliesslich den statischen `config.add`-Block
//! (dotted-Pfade werden automatisch angelegt, skips via `FieldExistsError`
//! aggregiert und als `SpecWarning::FieldExists` zurueckgegeben). Die
//! dynamischen URI-Quellen (HTTP(S)/Dateien mit `RefreshableGetter`-Caching
//! und Callbacks) sind reine Python-Seite und laufen ueber einen
//! Bridge-Callback am `PyProcessorCore` (`event.extra_data`-frei, siehe
//! `RegisteredRuleSpec::python_bridge`) — sie koennen Rust-paritätisch weder
//! beim `add_rule` (Content wird erst in `setup()` geladen) noch pro Event
//! ohne den Getter-Mechanismus gelöst werden.
//!
//! Logik-Mirror (aus `logprep/ng/processor/generic_adder/processor.py`):
//!
//! ```python
//! for items_to_add in rule.additions(event):  # config.add zuerst
//!     if items_to_add:
//!         add_fields_to(event, items_to_add, rule,
//!                       rule.merge_with_target, rule.overwrite_target,
//!                       skip_none=False)
//! ```

use pyo3::prelude::*;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::field::value::{add_fields_to, FieldExistsError};
use crate::filter::expression::pydict_to_json;

use super::{PyProcessorCore, PyRuleSpec, RuleSpec, SpecError, SpecWarning};

/// Statisch deserialisierte Rule-Config (nur `config.add` + Flags; die
/// URI-Quellen verarbeitet die Python-Bridge).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenericAdderSpecData {
    #[serde(default)]
    merge_with_target: bool,
    #[serde(default)]
    overwrite_target: bool,
    #[serde(default)]
    add: Map<String, Value>,
}

#[derive(Debug)]
pub struct GenericAdderRuleSpec {
    merge_with_target: bool,
    overwrite_target: bool,
    additions: Vec<(String, Value)>,
}

impl RuleSpec for GenericAdderRuleSpec {
    fn type_name(&self) -> &'static str {
        "generic_adder"
    }

    fn apply(&self, event: &mut Value, warnings: &mut Vec<SpecWarning>) -> Result<(), SpecError> {
        if self.additions.is_empty() {
            return Ok(());
        }
        match add_fields_to(
            event,
            &self.additions,
            self.merge_with_target,
            self.overwrite_target,
            false,
        ) {
            Ok(()) => Ok(()),
            Err(FieldExistsError { skipped_fields }) => {
                warnings.push(SpecWarning::FieldExists { skipped_fields });
                Ok(())
            }
        }
    }
}

/// Registriert einen `GenericAdderRuleSpec` am `PyProcessorCore`.
///
/// Wird vom ng-Adapter-Konstruktor einmal erzeugt und pro `add_rule`
/// (i.d.R. waehrend `load_rules`) mit den Rule-Config-Werten gerufen.
#[pyclass(name = "PyGenericAdderSpecFactory")]
pub struct PyGenericAdderSpecFactory;

#[pymethods]
impl PyGenericAdderSpecFactory {
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
        let data: GenericAdderSpecData = serde_json::from_value(Value::Object(raw))
            .map_err(|err| pyo3::exceptions::PyValueError::new_err(err.to_string()))?;
        let spec = GenericAdderRuleSpec {
            merge_with_target: data.merge_with_target,
            overwrite_target: data.overwrite_target,
            additions: data.add.into_iter().collect(),
        };
        core.set_rule_spec(py, rule_id, Py::new(py, PyRuleSpec::new(Box::new(spec)))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::expression::{FilterExpressionInner, PyFilterExpression};
    use crate::rule::PyRuleTree;
    use pyo3::types::{PyDict, PyList};
    use serde_json::json;

    fn spec_for(add: Value) -> GenericAdderRuleSpec {
        let data: GenericAdderSpecData = serde_json::from_value(add).unwrap();
        GenericAdderRuleSpec {
            merge_with_target: data.merge_with_target,
            overwrite_target: data.overwrite_target,
            additions: data.add.into_iter().collect(),
        }
    }

    fn apply(
        spec: &GenericAdderRuleSpec,
        event: &mut Value,
    ) -> (Result<(), SpecError>, Vec<SpecWarning>) {
        let mut warnings = Vec::new();
        let result = spec.apply(event, &mut warnings);
        (result, warnings)
    }

    #[test]
    fn writes_simple_add_fields() {
        let mut event = json!({"event_id": 123});
        let spec = spec_for(json!({
            "add": {
                "some_added_field": "some value",
                "another_added_field": "another_value",
            }
        }));
        let (result, warnings) = apply(&spec, &mut event);
        assert!(result.is_ok());
        assert!(warnings.is_empty());
        assert_eq!(
            event,
            json!({
                "event_id": 123,
                "some_added_field": "some value",
                "another_added_field": "another_value",
            })
        );
    }

    #[test]
    fn creates_dotted_path() {
        let mut event = json!({});
        let spec = spec_for(json!({
            "add": {"dotted.added.field": "yet_another_value"}
        }));
        let (result, _) = apply(&spec, &mut event);
        assert!(result.is_ok());
        assert_eq!(
            event,
            json!({"dotted": {"added": {"field": "yet_another_value"}}})
        );
    }

    #[test]
    fn writes_nested_and_list_values() {
        let mut event = json!({});
        let spec = spec_for(json!({
            "add": {
                "nested": {"a": 1},
                "items": ["x", 2, false],
                "flag": true,
            }
        }));
        let (result, _) = apply(&spec, &mut event);
        assert!(result.is_ok());
        assert_eq!(
            event,
            json!({
                "nested": {"a": 1},
                "items": ["x", 2, false],
                "flag": true,
            })
        );
    }

    #[test]
    fn merges_mapping_into_existing_target() {
        let mut event = json!({"shared_field": {"from_first_source": true}});
        let mut spec = spec_for(json!({
            "merge_with_target": true,
            "overwrite_target": false,
            "add": {"shared_field": {"from_inline_add": true}},
        }));
        spec.merge_with_target = true;
        let (result, warnings) = apply(&spec, &mut event);
        assert!(result.is_ok());
        assert!(warnings.is_empty());
        assert_eq!(
            event.get("shared_field"),
            Some(&json!({"from_first_source": true, "from_inline_add": true}))
        );
    }

    #[test]
    fn appends_to_existing_list_with_merge() {
        let mut event = json!({"items": [1, 2]});
        let mut spec = spec_for(json!({
            "merge_with_target": true,
            "add": {"items": [3]},
        }));
        spec.merge_with_target = true;
        let (result, _) = apply(&spec, &mut event);
        assert!(result.is_ok());
        assert_eq!(event.get("items"), Some(&json!([1, 2, 3])));
    }

    #[test]
    fn non_list_target_with_merge_warns() {
        // Mirror: failure_test_cases "Extend list field with 'merge_with_target'
        // enabled, but non-list target" — String-Content gegen String-Target
        // ohne overwrite_target bleibt eine Warnung.
        let mut event = json!({"items": "not_a_list"});
        let mut spec = spec_for(json!({
            "merge_with_target": true,
            "add": {"items": "some value"},
        }));
        spec.merge_with_target = true;
        let (result, warnings) = apply(&spec, &mut event);
        assert!(result.is_ok());
        assert_eq!(warnings.len(), 1);
        match &warnings[0] {
            SpecWarning::FieldExists { skipped_fields } => {
                assert_eq!(skipped_fields, &vec!["items".to_string()]);
            }
            other => panic!("expected FieldExists warning, got {other:?}"),
        }
        assert_eq!(event.get("items"), Some(&json!("not_a_list")));
    }

    #[test]
    fn overwrite_disabled_keeps_existing_value() {
        let mut event = json!({"some_added_field": "some_non_dict"});
        let spec = spec_for(json!({
            "overwrite_target": false,
            "add": {"some_added_field": "some value"},
        }));
        let (result, warnings) = apply(&spec, &mut event);
        assert!(result.is_ok());
        assert_eq!(warnings.len(), 1);
        match &warnings[0] {
            SpecWarning::FieldExists { skipped_fields } => {
                assert_eq!(skipped_fields, &vec!["some_added_field".to_string()]);
            }
            other => panic!("expected FieldExists warning, got {other:?}"),
        }
        assert_eq!(event.get("some_added_field"), Some(&json!("some_non_dict")));
    }

    #[test]
    fn overwrite_enabled_replaces_value() {
        let mut event = json!({"some_added_field": "old"});
        let mut spec = spec_for(json!({
            "overwrite_target": true,
            "add": {"some_added_field": "new"},
        }));
        spec.overwrite_target = true;
        let (result, warnings) = apply(&spec, &mut event);
        assert!(result.is_ok());
        assert!(warnings.is_empty());
        assert_eq!(event.get("some_added_field"), Some(&json!("new")));
    }

    #[test]
    fn partial_failure_writes_successful_fields_and_warns() {
        // Mirror: failure_test_cases "Add to existing value with
        // 'overwrite_target' disabled" — eines der Felder existiert bereits,
        // die uebrigen werden dennoch geschrieben.
        let mut event = json!({"some_added_field": "some_non_dict"});
        let spec = spec_for(json!({
            "overwrite_target": false,
            "add": {
                "some_added_field": "some value",
                "another_added_field": "another_value",
                "dotted.added.field": "yet_another_value",
            },
        }));
        let (result, warnings) = apply(&spec, &mut event);
        assert!(result.is_ok());
        assert_eq!(warnings.len(), 1);
        match &warnings[0] {
            SpecWarning::FieldExists { skipped_fields } => {
                assert_eq!(skipped_fields, &vec!["some_added_field".to_string()]);
            }
            other => panic!("expected FieldExists warning, got {other:?}"),
        }
        assert_eq!(
            event,
            json!({
                "some_added_field": "some_non_dict",
                "another_added_field": "another_value",
                "dotted": {"added": {"field": "yet_another_value"}},
            })
        );
    }

    #[test]
    fn empty_additions_is_noop() {
        let mut event = json!({"event_id": 1});
        let spec = spec_for(json!({"add": {}}));
        let (result, warnings) = apply(&spec, &mut event);
        assert!(result.is_ok());
        assert!(warnings.is_empty());
        assert_eq!(event, json!({"event_id": 1}));
    }

    #[test]
    fn additions_preserve_insertion_order() {
        let data: GenericAdderSpecData =
            serde_json::from_value(json!({"add": {"z": 1, "a": 2, "m": 3}})).unwrap();
        let keys: Vec<&str> = data
            .add
            .iter()
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>();
        assert_eq!(keys, vec!["z", "a", "m"]);
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
                'id': 'ga-fake',\n\
                'description': 'fake rule',\n\
            })"
        );
        py.eval(code, None, None).map(|t| t.unbind())
    }

    #[test]
    fn factory_requires_dict_rule_data() {
        py_ready();
        Python::with_gil(|py| {
            let factory = PyGenericAdderSpecFactory::new();
            let mut core = PyProcessorCore::new(false, false);
            let list = PyList::empty(py);
            let err = factory
                .make_and_register(py, &mut core, 1, list.as_any())
                .unwrap_err();
            assert!(err.to_string().contains("dict"));
        });
    }

    #[test]
    fn factory_registers_and_applies_on_matched_rule() {
        py_ready();
        Python::with_gil(|py| {
            let factory = PyGenericAdderSpecFactory::new();
            let mut core = PyProcessorCore::new(false, false);

            let tree = Py::new(py, PyRuleTree::new()).unwrap();
            add_to_tree(
                py,
                &tree,
                11,
                FilterExpressionInner::String {
                    key: vec!["f".into()],
                    expected: "v".into(),
                },
            )
            .unwrap();
            let rule = make_fake_rule(py).unwrap();
            let mapping = PyDict::new(py);
            mapping.set_item(11u64, &rule).unwrap();
            core.set_tree(tree, mapping.unbind());

            let rule_data = PyDict::new(py);
            rule_data
                .set_item("merge_with_target", false)
                .unwrap();
            rule_data
                .set_item("overwrite_target", false)
                .unwrap();
            let add = PyDict::new(py);
            add.set_item("added_field", "value").unwrap();
            add.set_item("dotted.path", true).unwrap();
            rule_data.set_item("add", &add).unwrap();
            factory
                .make_and_register(py, &mut core, 11, rule_data.as_any())
                .unwrap();

            let event = PyDict::new(py);
            event.set_item("f", "v").unwrap();
            let outcome = core.process(py, &event, None).unwrap();
            assert_eq!(outcome.matched_rule_ids, vec![11]);
            assert!(outcome.warnings.is_empty());
            assert!(outcome.errors.is_empty());
            let written = event.get_item("added_field").unwrap().unwrap();
            assert_eq!(written.extract::<String>().unwrap(), "value");
            let dotted = event.get_item("dotted").unwrap().unwrap();
            let dotted_dict = dotted.downcast::<PyDict>().unwrap();
            let path = dotted_dict.get_item("path").unwrap().unwrap();
            assert!(path.extract::<bool>().unwrap());
        });
    }
}