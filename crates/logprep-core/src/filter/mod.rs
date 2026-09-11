pub mod expression;
pub mod lucene;
pub mod range;

use pyo3::prelude::*;
use pyo3::wrap_pyfunction;

use self::expression::{
    filter_expression, filter_expression_and, filter_expression_exists,
    filter_expression_float, filter_expression_float_range, filter_expression_integer,
    filter_expression_integer_range, filter_expression_not, filter_expression_null,
    filter_expression_numeric_range, filter_expression_or, filter_expression_regex,
    filter_expression_sigma, filter_expression_string, filter_expression_string_range,
    filter_expression_wildcard, PyFilterExpression,
};
use self::lucene::parse_lucene_query;

/// Register all filter-related functions and classes on the parent module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyFilterExpression>()?;
    m.add_function(wrap_pyfunction!(filter_expression, m)?)?;
    m.add_function(wrap_pyfunction!(filter_expression_not, m)?)?;
    m.add_function(wrap_pyfunction!(filter_expression_and, m)?)?;
    m.add_function(wrap_pyfunction!(filter_expression_or, m)?)?;
    m.add_function(wrap_pyfunction!(filter_expression_string, m)?)?;
    m.add_function(wrap_pyfunction!(filter_expression_wildcard, m)?)?;
    m.add_function(wrap_pyfunction!(filter_expression_sigma, m)?)?;
    m.add_function(wrap_pyfunction!(filter_expression_integer, m)?)?;
    m.add_function(wrap_pyfunction!(filter_expression_float, m)?)?;
    m.add_function(wrap_pyfunction!(filter_expression_integer_range, m)?)?;
    m.add_function(wrap_pyfunction!(filter_expression_float_range, m)?)?;
    m.add_function(wrap_pyfunction!(filter_expression_numeric_range, m)?)?;
    m.add_function(wrap_pyfunction!(filter_expression_string_range, m)?)?;
    m.add_function(wrap_pyfunction!(filter_expression_regex, m)?)?;
    m.add_function(wrap_pyfunction!(filter_expression_exists, m)?)?;
    m.add_function(wrap_pyfunction!(filter_expression_null, m)?)?;
    m.add_function(wrap_pyfunction!(parse_lucene_query, m)?)?;
    Ok(())
}
