//! Field helpers: PyO3 wrappers for the legacy Python API (`py`) and the
//! pure-Rust `serde_json::Value` implementations used by the Rust rule specs
//! (`value`).

pub mod py;
pub mod value;

pub use py::{
    add_fields_to, field_list_to_dotted_field, get_dotted_field_list, get_dotted_field_value,
    get_dotted_field_value_with_missing, get_dotted_field_values, get_field_value,
    get_field_value_no_slice, has_dotted_field, join_dotted_fields, pop_dotted_field_value,
};
