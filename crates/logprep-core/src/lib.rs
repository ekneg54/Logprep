use pyo3::prelude::*;

pub mod field;
pub mod filter;
pub mod rule;

#[pymodule]
fn _rust(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Rule (Phase 3)
    rule::register(m)?;

    // Filter (Phase 2)
    filter::register(m)?;

    // Field (Phase 1)
    m.add_function(wrap_pyfunction!(field::get_dotted_field_list, m)?)?;
    m.add_function(wrap_pyfunction!(field::field_list_to_dotted_field, m)?)?;
    m.add_function(wrap_pyfunction!(field::join_dotted_fields, m)?)?;

    // Read
    m.add_function(wrap_pyfunction!(field::get_dotted_field_value, m)?)?;
    m.add_function(wrap_pyfunction!(
        field::get_dotted_field_value_with_missing,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(field::get_field_value, m)?)?;
    m.add_function(wrap_pyfunction!(field::get_field_value_no_slice, m)?)?;
    m.add_function(wrap_pyfunction!(field::get_dotted_field_values, m)?)?;

    // Existence
    m.add_function(wrap_pyfunction!(field::has_dotted_field, m)?)?;

    // Pop
    m.add_function(wrap_pyfunction!(field::pop_dotted_field_value, m)?)?;

    // Write
    m.add_function(wrap_pyfunction!(field::add_fields_to, m)?)?;

    Ok(())
}
