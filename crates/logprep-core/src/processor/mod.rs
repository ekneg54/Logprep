//! Processor-Orchestrierung (Phase 3.5).
//!
//! Migriert die Event-Verarbeitungs-Orchestrierung aus `logprep/ng/abc/processor.py`
//! nach Rust: `PyProcessorCore` ist der gemeinsame Ausfuehrungspfad fuer **alle**
//! ng-Prozessoren. Rule-Matching laeuft ueber den `RuleTree` (Phase 3); noch nicht
//! migrierte Prozessoren wenden ihre `_apply_rules`-Logik ueber einen Python-
//! Callback an (Phase 4 belegt dafuer den `rule_specs`-Slot mit pure-Rust-
//! Implementierungen).
//!
//! Das `ProcessOutcome` mit `matched_rule_ids` ist der definierte Seam fuer die
//! spaetere Rust-Metrics-Migration (Leitanforderung 3): Python inkrementiert die
//! Rule-Zaehler ausschliesslich ueber diese IDs.

pub mod core;
pub mod outcome;

use pyo3::prelude::*;

pub use self::core::PyProcessorCore;
pub use self::outcome::ProcessOutcome;

/// Pure-Rust Rule-Anwendung — deklariert in Phase 3.5, belegt erst in Phase 4.
///
/// Der `rule_specs`-Slot des `PyProcessorCore` bleibt in Phase 3.5 leer; sobald
/// ein Eintrag existiert, dispatcht der Core an `apply` statt an den Python-Callback.
pub trait RuleSpec: Send + Sync {
    fn apply(&self, event: &mut serde_json::Value) -> Result<(), String>;
}

/// Registriert die Processor-Klassen auf dem Submodul `logprep._rust.processor`
/// und haengt dieses in `sys.modules` ein, damit
/// `from logprep._rust.processor import PyProcessorCore` funktioniert.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let submodule = PyModule::new(m.py(), "logprep._rust.processor")?;
    submodule.add_class::<PyProcessorCore>()?;
    submodule.add_class::<ProcessOutcome>()?;
    m.add_submodule(&submodule)?;
    let sys = m.py().import("sys")?;
    let modules = sys.getattr("modules")?;
    modules.set_item("logprep._rust.processor", &submodule)?;
    Ok(())
}
