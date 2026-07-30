pub mod demorgan;
pub mod node;
pub mod parser;
pub mod segmenter;
pub mod sorter;
pub mod tagger;
pub mod tree;

use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::filter::expression::{
    pydict_to_json, FilterExpressionInner, PyFilterExpression,
};

use self::parser::RuleParserInner;
use self::tree::TreeInner;

#[pyclass]
pub struct PyRuleTree {
    inner: TreeInner,
}

#[pymethods]
impl PyRuleTree {
    #[new]
    fn new() -> Self {
        Self {
            inner: TreeInner::new(),
        }
    }

    #[pyo3(signature = (rule_id, segments))]
    fn add_rule(&mut self, rule_id: u64, segments: &Bound<'_, PyList>) -> PyResult<()> {
        for segment in segments.iter() {
            let segment_list = segment.downcast::<PyList>()?;
            let mut parsed = Vec::new();
            for item in segment_list.iter() {
                if let Ok(py_expr) = item.extract::<PyFilterExpression>() {
                    parsed.push(py_expr.inner.clone());
                } else {
                    let inner = FilterExpressionInner::from_py_object(&item)?;
                    parsed.push(inner);
                }
            }
            self.inner.add_rule(&parsed, rule_id);
        }
        Ok(())
    }

    fn get_matching_rules(&self, event: &Bound<'_, PyDict>) -> Vec<u64> {
        match pydict_to_json(event) {
            Ok(json_doc) => self.inner.get_matching_rules(&json_doc),
            Err(_) => Vec::new(),
        }
    }

    #[pyo3(signature = (filter_expr, priority_dict=None, tag_map=None))]
    fn parse_rule(
        &self,
        filter_expr: &Bound<'_, PyAny>,
        priority_dict: Option<HashMap<String, String>>,
        tag_map: Option<HashMap<String, String>>,
    ) -> PyResult<PyObject> {
        let inner = FilterExpressionInner::from_py_object(filter_expr)?;
        let priority = priority_dict.unwrap_or_default();
        let tags = tag_map.unwrap_or_default();
        let segments = RuleParserInner::parse(&inner, &priority, &tags);

        let py = filter_expr.py();
        let result = PyList::empty(py);
        for segment in &segments {
            let seg_list = PyList::empty(py);
            for expr in segment {
                let py_expr = PyFilterExpression {
                    inner: expr.clone(),
                };
                let py_any = py_expr.into_pyobject(py)?;
                seg_list.append(py_any)?;
            }
            result.append(seg_list)?;
        }
        Ok(result.into())
    }

    fn rule_count(&self) -> usize {
        self.inner.rule_count()
    }

    fn size(&self) -> usize {
        self.inner.size()
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyRuleTree>()?;
    Ok(())
}
