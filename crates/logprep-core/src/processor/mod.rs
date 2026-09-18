//! Processor-Orchestrierung (Phase 3.5 + 4b).
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
pub mod spec_helper;

use pyo3::prelude::*;
use serde_json::{Map, Value};

pub use self::core::PyProcessorCore;
pub use self::outcome::ProcessOutcome;

// ─── Spec-Types (Phase 4b) ─────────────────────────────────────────────────

/// Warnung, die ein `RuleSpec::apply` via `warnings`-Vektor zurueckgibt.
/// Wird vom Core in ein Python `ProcessingWarning` / `FieldExistsWarning`
/// konvertiert und ueber `handle_warning_error` in den `ProcessOutcome`
/// eingehaengt.
pub enum SpecWarning {
    /// Allgemeine Warnung (wird zu `ProcessingWarning(message, rule, event)`).
    Warning { message: String },
    /// Schreibfehler (wird zu `FieldExistsWarning(rule, event, skipped_fields)`).
    FieldExists { skipped_fields: Vec<String> },
}

/// Fehler, den ein `RuleSpec::apply` zurueckgeben kann (kritischer Pfad).
/// Wird vom Core in ein Python `ProcessingCriticalError` konvertiert.
pub enum SpecError {
    Critical { message: String },
}

/// Pure-Rust Rule-Anwendung — deklariert in Phase 3.5, belegt ab Phase 4.
///
/// Jeder migrierte Processor registriert einen Eintrag im `rule_specs`-Slot des
/// `PyProcessorCore`. Sobald ein Eintrag existiert, dispatcht der Core an `apply`
/// statt an den Python-Callback (`apply_hook`).
///
/// `apply` erhaelt das Event als `serde_json::Value` (fuer spec-interne
/// Hilfsaufrufe aus `field::value`) und einen Vektor von `SpecWarning`s.
/// Bei Erfolg (Ok(())) wertet der Core die Warnungen, synchronisiert das Value
/// zurueck ins Event-PyDict und fuehrt `delete_source_fields` aus.
pub trait RuleSpec: Send + Sync {
    /// Systemname des Specs (entspricht dem Processor-Type-Namen);
    /// fuer Diagnose-/Konfigurationsmeldungen.
    fn type_name(&self) -> &'static str;

    /// Einmalige Validierung beim Laden der Rule (z.B. Pflichtschluessel).
    /// Wird von der Python-Adapter-Klasse vor der Registrierung gerufen.
    /// `where Self: Sized`: statischer Aufruf auf konkreten Typen;
    /// Methodenaufruf ueber Trait-Objekte ist nicht noetig.
    fn validate(_raw: &Map<String, Value>) -> Result<(), String>
    where
        Self: Sized,
    {
        Ok(())
    }

    /// Wendet eine gematchte Rule auf das Event an.
    fn apply(&self, event: &mut Value, warnings: &mut Vec<SpecWarning>) -> Result<(), SpecError>;
}

// ─── PyO3-Wrapper fuer Box<dyn RuleSpec> ────────────────────────────────────

/// PyO3-Wrapper fuer ein `Box<dyn RuleSpec>`.  Wird von den Rust-Spec-Factory-
/// Klassen (z.B. `PyDropperSpecFactory`) erzeugt und dem Core ueber
/// `set_rule_spec(rule_id, spec)` uebergeben.
#[pyclass(name = "PyRuleSpec")]
pub struct PyRuleSpec {
    inner: Option<Box<dyn RuleSpec>>,
}

impl PyRuleSpec {
    /// Erzeugt einen neuen Wrapper (fuer Rust-Factory-Klassen).
    pub fn new(spec: Box<dyn RuleSpec>) -> Self {
        Self { inner: Some(spec) }
    }
}

// ─── Modul-Registrierung ───────────────────────────────────────────────────

/// Registriert die Processor-Klassen auf dem Submodul `logprep._rust.processor`
/// und haengt dieses in `sys.modules` ein, damit
/// `from logprep._rust.processor import PyProcessorCore` funktioniert.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let submodule = PyModule::new(m.py(), "logprep._rust.processor")?;
    submodule.add_class::<PyProcessorCore>()?;
    submodule.add_class::<ProcessOutcome>()?;
    submodule.add_class::<PyRuleSpec>()?;
    m.add_submodule(&submodule)?;
    let sys = m.py().import("sys")?;
    let modules = sys.getattr("modules")?;
    modules.set_item("logprep._rust.processor", &submodule)?;
    Ok(())
}
