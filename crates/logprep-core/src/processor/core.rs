//! `ProcessorCore` — Rust-Orchestrierung der Event-Verarbeitung (Phase 3.5).
//!
//! Migriert die bisherige Python-Orchestrierung aus `logprep/ng/abc/processor.py`:
//! Rule-Matching ueber den `RuleTree` (Phase 3), `apply_multiple_times`-Loop mit
//! Differenz-Menge, `data_error`-Skip, Warning-Tag-Merge (`_handle_warning_error`),
//! `delete_source_fields`-Aufraeumen und Exception-Klassifizierung
//! (`ProcessingWarning` / `ProcessingCriticalError` / generisch).
//!
//! Die eigentliche Rule-Anwendung (`_apply_rules`) laeuft fuer noch nicht
//! migrierte Prozessoren als Python-Callback (`apply_hook`). In Phase 4 wird der
//! `rule_specs`-Slot mit pure-Rust `RuleSpec`-Implementierungen belegt.

use std::collections::{BTreeSet, HashMap, HashSet};

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::filter::expression::{FilterExpressionInner, PyFilterExpression, pydict_to_json};
use crate::rule::PyRuleTree;

use super::RuleSpec;
use super::outcome::ProcessOutcome;

/// Gecachte, statische Metadaten einer Python-Rule.
///
/// Wird lazy beim ersten Zugriff aus dem Python-Objekt extrahiert.
/// `data_error` ist absichtlich **nicht** Teil des Caches — es kann sich zur
/// Laufzeit aendern (`Rule.mark_failed` / `clear_failed`) und wird pro Event
/// live am Python-Objekt abgefragt.
struct RuleMeta {
    /// Gesamter Filter der Rule (fuer den Bypass-Pfad, `rule.matches`-Aequivalent).
    filter: Option<FilterExpressionInner>,
    delete_source_fields: bool,
    source_fields: Vec<String>,
}

/// Extrahiert die statischen Metadaten aus einem Python-Rule-Objekt.
/// Entspricht den `getattr(rule, ..., default)`-Zugriffen im alten
/// `_apply_rules_wrapper` — fehlende Attribute fuehren zu Defaults.
fn extract_rule_meta(rule_obj: &Bound<'_, PyAny>) -> RuleMeta {
    let filter = rule_obj
        .getattr("filter")
        .ok()
        .and_then(|f| f.extract::<PyFilterExpression>().ok())
        .map(|f| f.inner);
    let delete_source_fields = rule_obj
        .getattr("delete_source_fields")
        .ok()
        .and_then(|v| v.extract::<bool>().ok())
        .unwrap_or(false);
    let source_fields = rule_obj
        .getattr("source_fields")
        .ok()
        .and_then(|v| v.extract::<Vec<String>>().ok())
        .unwrap_or_default();
    RuleMeta {
        filter,
        delete_source_fields,
        source_fields,
    }
}

/// Liest das `tags`-Feld eines Events als Menge einzelner Tag-Strings.
/// Spiegelt `tags_list = tags if isinstance(tags, list) else [tags]` aus
/// `_handle_warning_error`. Nicht-String-Elemente einer Liste werden — statt
/// wie in Python beim `sorted()` einen TypeError auszuloesen — ignoriert.
fn current_tag_set(event: &Bound<'_, PyDict>) -> PyResult<BTreeSet<String>> {
    let mut tag_set = BTreeSet::new();
    if let Some(tags) = event.get_item("tags")? {
        if let Ok(list) = tags.downcast::<PyList>() {
            for item in list.iter() {
                if let Ok(s) = item.extract::<String>() {
                    tag_set.insert(s);
                }
            }
        } else if let Ok(s) = tags.extract::<String>() {
            tag_set.insert(s);
        } else {
            tag_set.insert(tags.str()?.to_str()?.to_owned());
        }
    }
    Ok(tag_set)
}

/// PyO3-Adapter des `ProcessorCore`.
///
/// Haelt eine Referenz auf den Rust-`PyRuleTree` des Python-`RuleTree`-Wrappers
/// sowie auf dessen lebendes `_rule_id_to_rule`-Mapping. Dadurch wirken sich
/// nachtraegliche `RuleTree.add_rule(...)`-Aufrufe ohne erneutes Syncen aus;
/// `set_tree` (bei Ersetzen des RuleTree) invalidiert den Meta-Cache.
#[pyclass]
pub struct PyProcessorCore {
    tree: Option<Py<PyRuleTree>>,
    rule_mapping: Option<Py<PyDict>>,
    apply_multiple_times: bool,
    bypass_rule_tree: bool,
    meta_cache: HashMap<u64, RuleMeta>,
    /// Phase-4-Slot: pure-Rust Rule-Anwendung. In Phase 3.5 immer leer —
    /// alle Rules laufen ueber den Python-Callback-Pfad.
    rule_specs: HashMap<u64, Box<dyn RuleSpec>>,
}

#[pymethods]
impl PyProcessorCore {
    #[new]
    #[pyo3(signature = (apply_multiple_times=false, bypass_rule_tree=false))]
    fn new(apply_multiple_times: bool, bypass_rule_tree: bool) -> Self {
        Self {
            tree: None,
            rule_mapping: None,
            apply_multiple_times,
            bypass_rule_tree,
            meta_cache: HashMap::new(),
            rule_specs: HashMap::new(),
        }
    }

    /// Bindet den Rust-RuleTree und das lebendige Rule-Mapping des Python-
    /// `RuleTree`-Wrappers an. Invalidiert den Metadaten-Cache.
    fn set_tree(&mut self, tree: Py<PyRuleTree>, rule_mapping: Py<PyDict>) {
        self.tree = Some(tree);
        self.rule_mapping = Some(rule_mapping);
        self.meta_cache.clear();
    }

    /// Verarbeitet ein Event: Matcht Rules, wendet sie an (Callback oder
    /// Phase-4-RuleSpec) und sammelt das `ProcessOutcome`.
    ///
    /// `event` ist das lebende `event.data`-Dict — alle Mutationen (Callback,
    /// Tag-Merge, `delete_source_fields`) wirken direkt darauf.
    #[pyo3(signature = (event, apply_hook=None))]
    fn process(
        &mut self,
        py: Python<'_>,
        event: &Bound<'_, PyDict>,
        apply_hook: Option<PyObject>,
    ) -> PyResult<ProcessOutcome> {
        let mut outcome = ProcessOutcome::default();
        let mut value = pydict_to_json(event.as_any())?;
        let mut matched = self.matching_rule_ids(py, event, &value)?;
        let mut applied: HashSet<u64> = HashSet::new();
        loop {
            let batch: Vec<u64> = matched
                .iter()
                .filter(|id| !applied.contains(*id))
                .copied()
                .collect();
            if batch.is_empty() {
                break;
            }
            for rule_id in batch {
                applied.insert(rule_id);
                self.process_rule(py, event, rule_id, apply_hook.as_ref(), &mut outcome)?;
                outcome.matched_rule_ids.push(rule_id);
            }
            // Der Bypass-Pfad (`_process_all_rules` im alten Code) kennt kein
            // apply_multiple_times — er laeuft immer genau einmal.
            if !self.apply_multiple_times || self.bypass_rule_tree {
                break;
            }
            value = pydict_to_json(event.as_any())?;
            matched = self.matching_rule_ids(py, event, &value)?;
        }
        Ok(outcome)
    }
}

impl PyProcessorCore {
    /// Liefert die IDs der matchenden Rules — ueber den RuleTree oder, im
    /// Bypass-Modus (`LOGPREP_BYPASS_RULE_TREE`), durch direkten Filter-Match
    /// aller Rules in Einfuegereihenfolge (`_process_all_rules`-Aequivalent).
    fn matching_rule_ids(
        &mut self,
        py: Python<'_>,
        event: &Bound<'_, PyDict>,
        value: &serde_json::Value,
    ) -> PyResult<Vec<u64>> {
        if !self.bypass_rule_tree {
            let Some(tree) = &self.tree else {
                return Ok(Vec::new());
            };
            return Ok(tree.bind(py).borrow().get_matching_rules(event));
        }
        let Some(mapping) = &self.rule_mapping else {
            return Ok(Vec::new());
        };
        let pairs: Vec<(u64, PyObject)> = mapping
            .bind(py)
            .iter()
            .map(|(k, v)| Ok((k.extract::<u64>()?, v.unbind())))
            .collect::<PyResult<_>>()?;
        let mut matched = Vec::new();
        for (rule_id, rule_obj) in pairs {
            let meta = self.meta_for(rule_id, rule_obj.bind(py));
            if meta.filter.as_ref().is_some_and(|f| f.matches(value)) {
                matched.push(rule_id);
            }
        }
        Ok(matched)
    }

    /// Cached-Metadaten-Zugriff; extrahiert beim ersten Zugriff aus der Rule.
    fn meta_for(&mut self, rule_id: u64, rule_obj: &Bound<'_, PyAny>) -> &RuleMeta {
        self.meta_cache
            .entry(rule_id)
            .or_insert_with(|| extract_rule_meta(rule_obj))
    }

    /// Holt das Python-Rule-Objekt aus dem lebendigen Mapping.
    fn rule_object(&self, py: Python<'_>, rule_id: u64) -> PyResult<Option<PyObject>> {
        let Some(mapping) = &self.rule_mapping else {
            return Ok(None);
        };
        mapping
            .bind(py)
            .get_item(rule_id)
            .map(|opt| opt.map(|rule| rule.unbind()))
    }

    /// Verarbeitet eine einzelne Rule — das Rust-Aequivalent zu
    /// `_apply_rules_wrapper` + `_process_rule` (ohne Metrik, siehe Outcome-Vertrag).
    fn process_rule(
        &mut self,
        py: Python<'_>,
        event: &Bound<'_, PyDict>,
        rule_id: u64,
        apply_hook: Option<&PyObject>,
        outcome: &mut ProcessOutcome,
    ) -> PyResult<()> {
        let Some(rule_obj) = self.rule_object(py, rule_id)? else {
            return Ok(());
        };

        // data_error-Skip (bisher processor.py:176-180): kein apply, kein
        // delete_source_fields — nur Warning-Handling.
        let data_error = rule_obj.bind(py).getattr("data_error")?;
        if !data_error.is_none() {
            Self::handle_warning_error(py, event, rule_obj.bind(py), &data_error, outcome)?;
            return Ok(());
        }

        // Dispatch: Phase-4-RuleSpec-Slot (derzeit leer) oder Python-Callback.
        let apply_result = if self.rule_specs.contains_key(&rule_id) {
            // Phase 4: spec.apply(&mut value) — in Phase 3.5 unerreichbar,
            // da keine RuleSpecs registriert werden.
            Ok(())
        } else if let Some(hook) = apply_hook {
            hook.call1(py, (rule_id, event)).map(|_| ())
        } else {
            Ok(())
        };

        if let Err(err) = apply_result {
            Self::classify_error(py, event, rule_obj.bind(py), err, outcome)?;
        }

        // delete_source_fields-Aufraeumen (bisher processor.py:192-196) —
        // laeuft auf allen Pfaden ausser dem data_error-Skip.
        let (delete_source_fields, source_fields) = {
            let meta = self.meta_for(rule_id, rule_obj.bind(py));
            (meta.delete_source_fields, meta.source_fields.clone())
        };
        if delete_source_fields {
            for dotted_field in &source_fields {
                crate::field::pop_dotted_field_value(py, event.as_any(), dotted_field, true)?;
            }
        }
        Ok(())
    }

    /// Klassifiziert eine aus dem Callback geflohene Exception —
    /// entspricht den drei `except`-Bloecken des alten `_apply_rules_wrapper`.
    fn classify_error(
        py: Python<'_>,
        event: &Bound<'_, PyDict>,
        rule_obj: &Bound<'_, PyAny>,
        err: PyErr,
        outcome: &mut ProcessOutcome,
    ) -> PyResult<()> {
        let exceptions = py.import("logprep.processor.base.exceptions")?;
        let warning_cls = exceptions.getattr("ProcessingWarning")?;
        let critical_cls = exceptions.getattr("ProcessingCriticalError")?;
        if err.is_instance(py, &warning_cls) {
            let value = err.into_value(py);
            Self::handle_warning_error(py, event, rule_obj, value.bind(py).as_any(), outcome)?;
        } else if err.is_instance(py, &critical_cls) {
            outcome.errors.push(err.into_value(py).into_any());
        } else {
            // str(error) wie in `ProcessingCriticalError(str(error), rule)`
            let message = err.value(py).str()?.to_str()?.to_owned();
            let wrapped = critical_cls.call1((message, rule_obj))?;
            outcome.errors.push(wrapped.unbind());
        }
        Ok(())
    }

    /// Warning-Tag-Merge + Warning-Queue — das Rust-Aequivalent zu
    /// `_handle_warning_error` (bisher processor.py:227-251).
    ///
    /// Merged `rule.failure_tags` (und bei ProcessingWarnings zusaetzlich
    /// `error.tags`) sortiert und dedupliziert in `event["tags"]` und legt die
    /// (ggf. neu erzeugte) `ProcessingWarning` in `outcome.warnings` ab.
    /// `ProcessingWarning(...)` inkrementiert dabei — wie bisher — im
    /// Python-Konstruktor `rule.metrics.number_of_warnings`.
    fn handle_warning_error(
        py: Python<'_>,
        event: &Bound<'_, PyDict>,
        rule_obj: &Bound<'_, PyAny>,
        error: &Bound<'_, PyAny>,
        outcome: &mut ProcessOutcome,
    ) -> PyResult<()> {
        let exceptions = py.import("logprep.processor.base.exceptions")?;
        let warning_cls = exceptions.getattr("ProcessingWarning")?;

        let failure_tags: Vec<String> = rule_obj
            .getattr("failure_tags")
            .and_then(|v| v.extract::<Vec<String>>())
            .unwrap_or_default();

        // BTreeSet liefert sortiert + dedupliziert — entspricht
        // `sorted(list({*tags_list, *failure_tags}))`.
        let mut tag_set = current_tag_set(event)?;
        tag_set.extend(failure_tags.iter().cloned());
        let merged: Vec<String> = tag_set.iter().cloned().collect();
        event.set_item("tags", merged)?;

        if error.is_instance(&warning_cls)? {
            let error_tags: Vec<String> = error
                .getattr("tags")
                .and_then(|v| v.extract::<Vec<String>>())
                .unwrap_or_default();
            if !error_tags.is_empty() {
                tag_set.extend(error_tags);
                let merged: Vec<String> = tag_set.into_iter().collect();
                event.set_item("tags", merged)?;
            }
            outcome.warnings.push(error.clone().unbind());
        } else {
            // Generische Exception → ProcessingWarning(str(error), rule, event)
            let message = error.str()?.to_str()?.to_owned();
            let warning = warning_cls.call1((message, rule_obj, event))?;
            outcome.warnings.push(warning.unbind());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::expression::FilterExpressionInner;

    static LOGPREP_PATH_INIT: std::sync::Once = std::sync::Once::new();

    /// Stellt sicher, dass das Repo-Root auf `sys.path` liegt und dass
    /// `logprep._rust` aus dem Test-Binary (nicht aus der maturin-.so)
    /// registriert wird, damit pyclass-Typobjekte mit `extract::<...>()`
    /// uebereinstimmen.
    fn ensure_logprep_on_path(py: Python<'_>) {
        LOGPREP_PATH_INIT.call_once(|| {
            let repo_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
            let result = (|| -> PyResult<()> {
                let sys = py.import("sys")?;
                let path = sys.getattr("path")?;
                path.call_method1("insert", (0, repo_root))?;
                let modules = sys.getattr("modules")?.downcast_into::<PyDict>()?;
                if modules.get_item("logprep._rust")?.is_none() {
                    let module = PyModule::new(py, "logprep._rust")?;
                    crate::_rust(&module)?;
                    modules.set_item("logprep._rust", &module)?;
                }
                Ok(())
            })();
            if let Err(err) = result {
                panic!("failed to set sys.path for logprep: {err}");
            }
        });
    }

    /// Initialisiert den Python-Interpreter einmalig und präpariert `sys.path`.
    fn py_ready() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(ensure_logprep_on_path);
    }

    /// Baut eine minimale Python-Rule-Attrappe mit den Attributen,
    /// die der Core (und die Exception-Konstruktoren) auslesen:
    /// filter, data_error, failure_tags, metrics, id, description.
    fn make_rule_class(py: Python<'_>) -> PyResult<PyObject> {
        let code = pyo3::ffi::c_str!(
            "type('FakeMetrics', (), {'number_of_errors': 0, 'number_of_warnings': 0, 'number_of_processed_events': 0})()"
        );
        let metrics = py.eval(code, None, None)?;
        let rule_code = pyo3::ffi::c_str!(
            "type('FakeRule', (), {\n\
                'filter': None,\n\
                'data_error': None,\n\
                'failure_tags': ['_fake_failure'],\n\
                'delete_source_fields': False,\n\
                'source_fields': [],\n\
                'id': 'fake-rule-id',\n\
                'description': 'fake rule',\n\
            })"
        );
        let rule_type = py.eval(rule_code, None, None)?;
        rule_type.setattr("metrics", metrics)?;
        let filter_mod = PyModule::import(py, "logprep.filter.expression")?;
        let always = filter_mod.getattr("Always")?.call1((true,))?;
        rule_type.setattr("filter", always)?;
        Ok(rule_type.unbind())
    }

    fn make_rule(py: Python<'_>, rule_type: &PyObject) -> PyResult<PyObject> {
        rule_type.call0(py)
    }

    /// Registriert eine Rule direkt im Rust-Tree und im Mapping-Dict.
    fn setup_core_with_rule(
        py: Python<'_>,
        core: &mut PyProcessorCore,
        rule_id: u64,
        expr: FilterExpressionInner,
        rule_obj: &PyObject,
    ) -> PyResult<()> {
        let tree = Py::new(py, PyRuleTree::new())?;
        add_to_tree(py, &tree, rule_id, expr)?;
        let mapping = PyDict::new(py);
        mapping.set_item(rule_id, rule_obj)?;
        core.set_tree(tree, mapping.unbind());
        Ok(())
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
        tree.bind(py).borrow_mut().add_rule(rule_id, &segments)?;
        Ok(())
    }

    fn new_core(apply_multiple_times: bool, bypass: bool) -> PyProcessorCore {
        PyProcessorCore::new(apply_multiple_times, bypass)
    }

    fn event_dict<'py>(py: Python<'py>, pairs: &[(&str, &str)]) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        for (k, v) in pairs {
            dict.set_item(*k, *v)?;
        }
        Ok(dict)
    }

    fn string_expr(key: &str, expected: &str) -> FilterExpressionInner {
        FilterExpressionInner::String {
            key: vec![key.into()],
            expected: expected.into(),
        }
    }

    fn always_expr() -> FilterExpressionInner {
        FilterExpressionInner::Always { value: true }
    }

    fn hook_from_code(py: Python<'_>, code: &str) -> PyResult<PyObject> {
        let c_code = std::ffi::CString::new(code).unwrap();
        let module = PyModule::from_code(
            py,
            &c_code,
            pyo3::ffi::c_str!("hook_mod"),
            pyo3::ffi::c_str!("hook_mod"),
        )?;
        Ok(module.getattr("hook")?.unbind())
    }

    /// apply_hook, das nichts tut (Erfolgspfad).
    fn noop_hook(py: Python<'_>) -> PyResult<PyObject> {
        hook_from_code(py, "def hook(rule_id, event):\n    pass")
    }

    /// apply_hook, das ein Feld setzt.
    fn set_field_hook(py: Python<'_>, field: &str, value: &str) -> PyResult<PyObject> {
        hook_from_code(
            py,
            &format!("def hook(rule_id, event):\n    event['{field}'] = '{value}'"),
        )
    }

    /// apply_hook, das eine gegebene Exception wirft.
    fn raising_hook(py: Python<'_>, exc_expr: &str) -> PyResult<PyObject> {
        hook_from_code(
            py,
            &format!(
                "from logprep.processor.base.exceptions import ProcessingWarning, ProcessingCriticalError\n\
                 def hook(rule_id, event):\n    raise {exc_expr}"
            ),
        )
    }

    #[test]
    fn test_process_matches_and_collects_rule_ids() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            setup_core_with_rule(py, &mut core, 7, string_expr("f", "v"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            let outcome = core
                .process(py, &event, Some(noop_hook(py).unwrap()))
                .unwrap();
            assert_eq!(outcome.matched_rule_ids, vec![7]);
            assert!(outcome.warnings.is_empty());
            assert!(outcome.errors.is_empty());
        });
    }

    #[test]
    fn test_process_no_match_empty_outcome() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            setup_core_with_rule(py, &mut core, 7, string_expr("f", "other"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            let outcome = core
                .process(py, &event, Some(noop_hook(py).unwrap()))
                .unwrap();
            assert!(outcome.matched_rule_ids.is_empty());
        });
    }

    #[test]
    fn test_process_without_hook_still_reports_matched_ids() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            setup_core_with_rule(py, &mut core, 3, string_expr("f", "v"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            let outcome = core.process(py, &event, None).unwrap();
            assert_eq!(outcome.matched_rule_ids, vec![3]);
        });
    }

    #[test]
    fn test_process_without_tree_is_noop() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let event = event_dict(py, &[("f", "v")]).unwrap();
            let outcome = core
                .process(py, &event, Some(noop_hook(py).unwrap()))
                .unwrap();
            assert!(outcome.matched_rule_ids.is_empty());
        });
    }

    #[test]
    fn test_process_without_tree_bypass_is_noop() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, true);
            let event = event_dict(py, &[("f", "v")]).unwrap();
            let outcome = core
                .process(py, &event, Some(noop_hook(py).unwrap()))
                .unwrap();
            assert!(outcome.matched_rule_ids.is_empty());
        });
    }

    #[test]
    fn test_callback_mutates_event() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            setup_core_with_rule(py, &mut core, 1, string_expr("f", "v"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            let hook = set_field_hook(py, "added", "yes").unwrap();
            core.process(py, &event, Some(hook)).unwrap();
            let added: String = event.get_item("added").unwrap().unwrap().extract().unwrap();
            assert_eq!(added, "yes");
        });
    }

    #[test]
    fn test_callback_receives_rule_id_and_event() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            setup_core_with_rule(py, &mut core, 42, string_expr("f", "v"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            let hook = hook_from_code(
                py,
                "def hook(rule_id, event):\n    event['seen_id'] = rule_id",
            )
            .unwrap();
            core.process(py, &event, Some(hook)).unwrap();
            let seen: u64 = event.get_item("seen_id").unwrap().unwrap().extract().unwrap();
            assert_eq!(seen, 42);
        });
    }

    #[test]
    fn test_processing_warning_merges_failure_tags() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            setup_core_with_rule(py, &mut core, 1, string_expr("f", "v"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            let hook = raising_hook(py, "ProcessingWarning('boom', None, event)").unwrap();
            let outcome = core.process(py, &event, Some(hook)).unwrap();
            assert_eq!(outcome.warnings.len(), 1);
            assert!(outcome.errors.is_empty());
            // Metrik-Vertrag: matched_rule_ids enthaelt die Rule trotz Warning
            assert_eq!(outcome.matched_rule_ids, vec![1]);
            let tags: Vec<String> = event.get_item("tags").unwrap().unwrap().extract().unwrap();
            assert_eq!(tags, vec!["_fake_failure".to_string()]);
        });
    }

    #[test]
    fn test_processing_warning_merges_existing_and_error_tags() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            setup_core_with_rule(py, &mut core, 1, string_expr("f", "v"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            event
                .set_item("tags", vec!["existing".to_string(), "another".to_string()])
                .unwrap();
            let hook = raising_hook(
                py,
                "ProcessingWarning('boom', None, event, tags=['_extra'])",
            )
            .unwrap();
            let outcome = core.process(py, &event, Some(hook)).unwrap();
            assert_eq!(outcome.warnings.len(), 1);
            let tags: Vec<String> = event.get_item("tags").unwrap().unwrap().extract().unwrap();
            assert_eq!(
                tags,
                vec![
                    "_extra".to_string(),
                    "_fake_failure".to_string(),
                    "another".to_string(),
                    "existing".to_string()
                ]
            );
        });
    }

    #[test]
    fn test_existing_tags_as_string_are_merged() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            setup_core_with_rule(py, &mut core, 1, string_expr("f", "v"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            event.set_item("tags", "single_tag").unwrap();
            let hook = raising_hook(py, "ProcessingWarning('boom', None, event)").unwrap();
            core.process(py, &event, Some(hook)).unwrap();
            let tags: Vec<String> = event.get_item("tags").unwrap().unwrap().extract().unwrap();
            assert_eq!(
                tags,
                vec!["_fake_failure".to_string(), "single_tag".to_string()]
            );
        });
    }

    #[test]
    fn test_critical_error_goes_to_errors_not_warnings() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            setup_core_with_rule(py, &mut core, 1, string_expr("f", "v"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            // Hook, das ProcessingCriticalError mit der Rule wirft (via Modul-Global)
            let c_code = std::ffi::CString::new(
                "from logprep.processor.base.exceptions import ProcessingCriticalError\n\
                 def hook(rule_id, event):\n    raise ProcessingCriticalError('kaputt', rule)",
            )
            .unwrap();
            let module = PyModule::from_code(
                py,
                &c_code,
                pyo3::ffi::c_str!("hook_mod"),
                pyo3::ffi::c_str!("hook_mod"),
            )
            .unwrap();
            module.setattr("rule", &rule).unwrap();
            let hook = module.getattr("hook").unwrap().unbind();
            let outcome = core.process(py, &event, Some(hook)).unwrap();
            assert_eq!(outcome.errors.len(), 1);
            assert!(outcome.warnings.is_empty());
            // tags wurden nicht gesetzt (kein Warning-Handling)
            assert!(event.get_item("tags").unwrap().is_none());
            // Metrik-Vertrag: Rule wurde verarbeitet
            assert_eq!(outcome.matched_rule_ids, vec![1]);
        });
    }

    #[test]
    fn test_classify_error_processing_critical_error_directly() {
        py_ready();
        Python::with_gil(|py| {
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            let exceptions = PyModule::import(py, "logprep.processor.base.exceptions").unwrap();
            let critical = exceptions
                .getattr("ProcessingCriticalError")
                .unwrap()
                .call1(("kaputt", &rule))
                .unwrap();
            let err = PyErr::from_value(critical);
            let mut outcome = ProcessOutcome::default();
            PyProcessorCore::classify_error(py, &event, rule.bind(py), err, &mut outcome).unwrap();
            assert_eq!(outcome.errors.len(), 1);
            assert!(outcome.warnings.is_empty());
        });
    }

    #[test]
    fn test_generic_exception_is_wrapped_as_critical_error() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            setup_core_with_rule(py, &mut core, 1, string_expr("f", "v"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            let hook = raising_hook(py, "ValueError('inner')").unwrap();
            let outcome = core.process(py, &event, Some(hook)).unwrap();
            assert_eq!(outcome.errors.len(), 1);
            assert!(outcome.warnings.is_empty());
            let err_str = outcome.errors[0].bind(py).str().unwrap();
            let err_str = err_str.to_str().unwrap();
            assert!(err_str.contains("inner"), "unexpected: {err_str}");
            // Rule wurde trotzdem verarbeitet (Metrik-Vertrag)
            assert_eq!(outcome.matched_rule_ids, vec![1]);
        });
    }

    #[test]
    fn test_data_error_skips_apply_and_warns() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            let data_error = pyo3::exceptions::PyRuntimeError::new_err("getter failed")
                .into_value(py);
            rule.bind(py).setattr("data_error", data_error).unwrap();
            setup_core_with_rule(py, &mut core, 1, string_expr("f", "v"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            let hook = set_field_hook(py, "must_not_be_set", "x").unwrap();
            let outcome = core.process(py, &event, Some(hook)).unwrap();
            // apply wurde uebersprungen
            assert!(event.get_item("must_not_be_set").unwrap().is_none());
            // Warning wurde erzeugt (ProcessingWarning(str(error), rule, event))
            assert_eq!(outcome.warnings.len(), 1);
            // Metrik zaehlt trotzdem (alter _process_rule-Vertrag)
            assert_eq!(outcome.matched_rule_ids, vec![1]);
            // failure_tags wurden gemerged
            let tags: Vec<String> = event.get_item("tags").unwrap().unwrap().extract().unwrap();
            assert_eq!(tags, vec!["_fake_failure".to_string()]);
        });
    }

    #[test]
    fn test_data_error_skips_delete_source_fields() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            rule.bind(py).setattr("delete_source_fields", true).unwrap();
            rule.bind(py).setattr("source_fields", vec!["f"]).unwrap();
            let data_error = pyo3::exceptions::PyRuntimeError::new_err("x").into_value(py);
            rule.bind(py).setattr("data_error", data_error).unwrap();
            setup_core_with_rule(py, &mut core, 1, string_expr("f", "v"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            core.process(py, &event, Some(noop_hook(py).unwrap()))
                .unwrap();
            // data_error-Pfad: delete_source_fields wird NICHT ausgefuehrt
            assert!(event.get_item("f").unwrap().is_some());
        });
    }

    #[test]
    fn test_delete_source_fields_pops_fields() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            rule.bind(py).setattr("delete_source_fields", true).unwrap();
            rule.bind(py)
                .setattr("source_fields", vec!["f", "nested.x"])
                .unwrap();
            setup_core_with_rule(py, &mut core, 1, string_expr("f", "v"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v"), ("keep", "1")]).unwrap();
            let nested = PyDict::new(py);
            nested.set_item("x", "y").unwrap();
            nested.set_item("z", "w").unwrap();
            event.set_item("nested", nested).unwrap();
            let outcome = core
                .process(py, &event, Some(noop_hook(py).unwrap()))
                .unwrap();
            assert_eq!(outcome.matched_rule_ids, vec![1]);
            assert!(event.get_item("f").unwrap().is_none());
            assert!(event.get_item("keep").unwrap().is_some());
            // drop_empty=true entfernt "x", behaelt aber "z" im verschachtelten Dict
            let nested = event.get_item("nested").unwrap().unwrap();
            let nested = nested.downcast::<PyDict>().unwrap();
            assert!(nested.get_item("x").unwrap().is_none());
            assert!(nested.get_item("z").unwrap().is_some());
        });
    }

    #[test]
    fn test_delete_source_fields_drops_empty_parents() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            rule.bind(py).setattr("delete_source_fields", true).unwrap();
            rule.bind(py)
                .setattr("source_fields", vec!["nested.only"])
                .unwrap();
            setup_core_with_rule(py, &mut core, 1, string_expr("f", "v"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            let nested = PyDict::new(py);
            nested.set_item("only", "x").unwrap();
            event.set_item("nested", nested).unwrap();
            core.process(py, &event, Some(noop_hook(py).unwrap()))
                .unwrap();
            // leeres Eltern-Dict wurde mit entfernt
            assert!(event.get_item("nested").unwrap().is_none());
        });
    }

    #[test]
    fn test_delete_source_fields_runs_after_warning() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            rule.bind(py).setattr("delete_source_fields", true).unwrap();
            rule.bind(py).setattr("source_fields", vec!["f"]).unwrap();
            setup_core_with_rule(py, &mut core, 1, string_expr("f", "v"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            let hook = raising_hook(py, "ProcessingWarning('boom', None, event)").unwrap();
            let outcome = core.process(py, &event, Some(hook)).unwrap();
            assert_eq!(outcome.warnings.len(), 1);
            // Warning-Pfad: delete_source_fields laeuft trotzdem
            assert!(event.get_item("f").unwrap().is_none());
        });
    }

    #[test]
    fn test_apply_multiple_times_reprocesses_changed_event() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(true, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule1 = make_rule(py, &rule_type).unwrap();
            setup_core_with_rule(py, &mut core, 1, always_expr(), &rule1).unwrap();
            // Rule 2 ans selbe Tree/Mapping haengen (ohne set_tree)
            let tree = core.tree.as_ref().unwrap().clone_ref(py);
            let mapping = core.rule_mapping.as_ref().unwrap().clone_ref(py);
            let rule2 = make_rule(py, &rule_type).unwrap();
            add_to_tree(py, &tree, 2, string_expr("generated", "yes")).unwrap();
            mapping.bind(py).set_item(2u64, &rule2).unwrap();
            let event = event_dict(py, &[("start", "1")]).unwrap();
            // Hook: schreibt "generated" nur einmal (idempotent)
            let hook = hook_from_code(
                py,
                "def hook(rule_id, event):\n    event['generated'] = 'yes'",
            )
            .unwrap();
            let outcome = core.process(py, &event, Some(hook)).unwrap();
            // Beide Rules wurden genau einmal angewendet (Differenz-Menge)
            assert_eq!(outcome.matched_rule_ids, vec![1, 2]);
        });
    }

    #[test]
    fn test_apply_multiple_times_rule_applies_only_once() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(true, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            setup_core_with_rule(py, &mut core, 5, always_expr(), &rule).unwrap();
            let event = event_dict(py, &[("start", "1")]).unwrap();
            let outcome = core
                .process(py, &event, Some(noop_hook(py).unwrap()))
                .unwrap();
            // Always-Rule matcht in jeder Runde, darf aber nur einmal zaehlen
            assert_eq!(outcome.matched_rule_ids, vec![5]);
        });
    }

    #[test]
    fn test_apply_multiple_times_terminates_without_new_matches() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(true, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            setup_core_with_rule(py, &mut core, 5, string_expr("f", "v"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            let outcome = core
                .process(py, &event, Some(noop_hook(py).unwrap()))
                .unwrap();
            assert_eq!(outcome.matched_rule_ids, vec![5]);
        });
    }

    #[test]
    fn test_bypass_matches_all_rules_via_filter() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, true);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            // Tree-Segmente wuerden nicht matchen — Bypass nutzt rule.filter (Always)
            setup_core_with_rule(py, &mut core, 9, string_expr("no", "match"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            let outcome = core
                .process(py, &event, Some(noop_hook(py).unwrap()))
                .unwrap();
            assert_eq!(outcome.matched_rule_ids, vec![9]);
        });
    }

    #[test]
    fn test_bypass_non_matching_filter_excluded() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, true);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            setup_core_with_rule(py, &mut core, 9, string_expr("no", "match"), &rule).unwrap();
            // Filter der FakeRule auf etwas Nicht-Matchendes setzen
            let filter_mod = PyModule::import(py, "logprep.filter.expression").unwrap();
            let never = filter_mod
                .getattr("StringFilterExpression")
                .unwrap()
                .call1((vec!["no".to_string()], "match"))
                .unwrap();
            rule.bind(py).setattr("filter", never).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            let outcome = core
                .process(py, &event, Some(noop_hook(py).unwrap()))
                .unwrap();
            assert!(outcome.matched_rule_ids.is_empty());
        });
    }

    #[test]
    fn test_bypass_runs_single_pass_even_with_apply_multiple_times() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(true, true);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            setup_core_with_rule(py, &mut core, 9, string_expr("no", "match"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            let hook = hook_from_code(
                py,
                "def hook(rule_id, event):\n    event['count'] = event.get('count', 0) + 1",
            )
            .unwrap();
            let outcome = core.process(py, &event, Some(hook)).unwrap();
            assert_eq!(outcome.matched_rule_ids, vec![9]);
            let count: u64 = event.get_item("count").unwrap().unwrap().extract().unwrap();
            assert_eq!(count, 1);
        });
    }

    #[test]
    fn test_set_tree_invalidates_meta_cache() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            setup_core_with_rule(py, &mut core, 1, string_expr("f", "v"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            core.process(py, &event, Some(noop_hook(py).unwrap()))
                .unwrap();
            assert!(!core.meta_cache.is_empty());
            // Neuer Tree → Cache geleert
            let tree = Py::new(py, PyRuleTree::new()).unwrap();
            let mapping = PyDict::new(py);
            core.set_tree(tree, mapping.unbind());
            assert!(core.meta_cache.is_empty());
            let outcome = core
                .process(py, &event, Some(noop_hook(py).unwrap()))
                .unwrap();
            assert!(outcome.matched_rule_ids.is_empty());
        });
    }

    #[test]
    fn test_late_added_rule_is_visible_without_resync() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            setup_core_with_rule(py, &mut core, 1, string_expr("f", "v"), &rule).unwrap();
            // Spaeter hinzugefuegte Rule direkt ueber Tree+Mapping (ohne set_tree)
            let tree = core.tree.as_ref().unwrap().clone_ref(py);
            let mapping = core.rule_mapping.as_ref().unwrap().clone_ref(py);
            let rule2 = make_rule(py, &rule_type).unwrap();
            add_to_tree(py, &tree, 2, string_expr("g", "w")).unwrap();
            mapping.bind(py).set_item(2u64, &rule2).unwrap();
            let event = event_dict(py, &[("f", "v"), ("g", "w")]).unwrap();
            let outcome = core
                .process(py, &event, Some(noop_hook(py).unwrap()))
                .unwrap();
            assert!(outcome.matched_rule_ids.contains(&1));
            assert!(outcome.matched_rule_ids.contains(&2));
        });
    }

    #[test]
    fn test_multiple_rules_all_matched() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let tree = Py::new(py, PyRuleTree::new()).unwrap();
            let mapping = PyDict::new(py);
            for id in [10u64, 20, 30] {
                let rule = make_rule(py, &rule_type).unwrap();
                add_to_tree(py, &tree, id, always_expr()).unwrap();
                mapping.set_item(id, rule).unwrap();
            }
            core.set_tree(tree, mapping.unbind());
            let event = event_dict(py, &[("a", "b")]).unwrap();
            let outcome = core
                .process(py, &event, Some(noop_hook(py).unwrap()))
                .unwrap();
            assert_eq!(outcome.matched_rule_ids.len(), 3);
            for id in [10u64, 20, 30] {
                assert!(outcome.matched_rule_ids.contains(&id));
            }
        });
    }

    #[test]
    fn test_processing_continues_after_warning() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let tree = Py::new(py, PyRuleTree::new()).unwrap();
            let mapping = PyDict::new(py);
            for id in [1u64, 2] {
                let rule = make_rule(py, &rule_type).unwrap();
                add_to_tree(py, &tree, id, always_expr()).unwrap();
                mapping.set_item(id, rule).unwrap();
            }
            core.set_tree(tree, mapping.unbind());
            let event = event_dict(py, &[("a", "b")]).unwrap();
            let hook = hook_from_code(
                py,
                "from logprep.processor.base.exceptions import ProcessingWarning\n\
                 def hook(rule_id, event):\n\
                 \x20   if rule_id == 1:\n\
                 \x20       raise ProcessingWarning('w', None, event)\n\
                 \x20   event['done'] = True",
            )
            .unwrap();
            let outcome = core.process(py, &event, Some(hook)).unwrap();
            assert_eq!(outcome.warnings.len(), 1);
            assert_eq!(outcome.matched_rule_ids.len(), 2);
            assert!(event.get_item("done").unwrap().is_some());
        });
    }

    #[test]
    fn test_processing_continues_after_critical_error() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let tree = Py::new(py, PyRuleTree::new()).unwrap();
            let mapping = PyDict::new(py);
            for id in [1u64, 2] {
                let rule = make_rule(py, &rule_type).unwrap();
                add_to_tree(py, &tree, id, always_expr()).unwrap();
                mapping.set_item(id, rule).unwrap();
            }
            core.set_tree(tree, mapping.unbind());
            let event = event_dict(py, &[("a", "b")]).unwrap();
            let hook = hook_from_code(
                py,
                "def hook(rule_id, event):\n\
                 \x20   if rule_id == 1:\n\
                 \x20       raise ValueError('bad')\n\
                 \x20   event['done'] = True",
            )
            .unwrap();
            let outcome = core.process(py, &event, Some(hook)).unwrap();
            assert_eq!(outcome.errors.len(), 1);
            assert_eq!(outcome.matched_rule_ids.len(), 2);
            assert!(event.get_item("done").unwrap().is_some());
        });
    }

    #[test]
    fn test_missing_rule_in_mapping_is_skipped() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            setup_core_with_rule(py, &mut core, 1, string_expr("f", "v"), &rule).unwrap();
            // Mapping-Eintrag entfernen — Tree hat die Rule noch
            core.rule_mapping
                .as_ref()
                .unwrap()
                .bind(py)
                .del_item(1u64)
                .unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            let outcome = core
                .process(py, &event, Some(noop_hook(py).unwrap()))
                .unwrap();
            // Rule wurde gematcht, aber ohne Mapping-Eintrag still uebersprungen
            assert_eq!(outcome.matched_rule_ids, vec![1]);
            assert!(outcome.errors.is_empty());
        });
    }

    #[test]
    fn test_rule_specs_slot_exists_and_is_empty() {
        let core = new_core(false, false);
        assert!(core.rule_specs.is_empty());
    }

    #[test]
    fn test_extract_rule_meta_defaults() {
        py_ready();
        Python::with_gil(|py| {
            let obj = PyDict::new(py);
            let meta = extract_rule_meta(obj.as_any());
            assert!(meta.filter.is_none());
            assert!(!meta.delete_source_fields);
            assert!(meta.source_fields.is_empty());
        });
    }

    #[test]
    fn test_extract_rule_meta_reads_attributes() {
        py_ready();
        Python::with_gil(|py| {
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            rule.bind(py).setattr("delete_source_fields", true).unwrap();
            rule.bind(py).setattr("source_fields", vec!["a.b"]).unwrap();
            let meta = extract_rule_meta(rule.bind(py));
            assert!(meta.filter.is_some());
            assert!(meta.delete_source_fields);
            assert_eq!(meta.source_fields, vec!["a.b".to_string()]);
        });
    }

    #[test]
    fn test_extract_rule_meta_ignores_wrong_types() {
        py_ready();
        Python::with_gil(|py| {
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            rule.bind(py)
                .setattr("delete_source_fields", "not-a-bool")
                .unwrap();
            rule.bind(py).setattr("source_fields", 42).unwrap();
            let meta = extract_rule_meta(rule.bind(py));
            assert!(!meta.delete_source_fields);
            assert!(meta.source_fields.is_empty());
        });
    }

    #[test]
    fn test_current_tag_set_variants() {
        py_ready();
        Python::with_gil(|py| {
            let event = PyDict::new(py);
            assert!(current_tag_set(&event).unwrap().is_empty());
            event.set_item("tags", "single").unwrap();
            let set = current_tag_set(&event).unwrap();
            assert!(set.contains("single"));
            event
                .set_item("tags", vec!["a".to_string(), "b".to_string()])
                .unwrap();
            let set = current_tag_set(&event).unwrap();
            assert!(set.contains("a") && set.contains("b"));
            // Nicht-String/Nicht-Liste-Fallback: str()-Repraesentation
            event.set_item("tags", 42).unwrap();
            let set = current_tag_set(&event).unwrap();
            assert!(set.contains("42"));
            // Liste mit Nicht-Strings: nur String-Elemente
            event
                .set_item("tags", vec![
                    "x".into_pyobject(py).unwrap().into_any().unbind(),
                    7u64.into_pyobject(py).unwrap().into_any().unbind(),
                ])
                .unwrap();
            let set = current_tag_set(&event).unwrap();
            assert!(set.contains("x") && !set.contains("7"));
        });
    }

    #[test]
    fn test_warning_dedupes_tags() {
        py_ready();
        Python::with_gil(|py| {
            let mut core = new_core(false, false);
            let rule_type = make_rule_class(py).unwrap();
            let rule = make_rule(py, &rule_type).unwrap();
            setup_core_with_rule(py, &mut core, 1, string_expr("f", "v"), &rule).unwrap();
            let event = event_dict(py, &[("f", "v")]).unwrap();
            event
                .set_item("tags", vec!["_fake_failure".to_string()])
                .unwrap();
            let hook = raising_hook(py, "ProcessingWarning('boom', None, event)").unwrap();
            core.process(py, &event, Some(hook)).unwrap();
            let tags: Vec<String> = event.get_item("tags").unwrap().unwrap().extract().unwrap();
            assert_eq!(tags, vec!["_fake_failure".to_string()]);
        });
    }

    #[test]
    fn test_outcome_default_is_empty() {
        let outcome = ProcessOutcome::default();
        assert!(outcome.matched_rule_ids.is_empty());
        assert!(outcome.warnings.is_empty());
        assert!(outcome.errors.is_empty());
    }
}
