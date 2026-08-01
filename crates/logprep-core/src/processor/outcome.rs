//! Rueckgabetypen fuer die Processor-Orchestrierung (Phase 3.5).

use pyo3::prelude::*;

/// Ergebnis eines `ProcessorCore.process()`-Aufrufs.
///
/// `matched_rule_ids` ist der **einzige** Weg, wie Python an die getroffenen
/// Rules gelangt (z.B. fuer `Rule.metrics.number_of_processed_events`).
/// Das ist der dokumentierte Seam fuer die spaetere Rust-Metrics-Migration
/// (Anforderung 3): dann uebernimmt ein Rust-Metrics-Registry die Zaehler
/// und `Rule.metrics` wird zum duennen PyO3-Blick darauf.
#[pyclass]
#[derive(Default)]
pub struct ProcessOutcome {
    /// IDs aller Rules, die waehrend `process()` getroffen und angewendet wurden
    /// (inkl. ueber `data_error` uebersprungener Rules — siehe `_process_rule`
    /// im alten `processor.py`, das die Metrik ebenfalls inkrementierte).
    #[pyo3(get)]
    pub matched_rule_ids: Vec<u64>,
    /// `ProcessingWarning`-Instanzen, in Reihenfolge ihres Auftretens.
    /// Python haengt sie an `event.warnings` an.
    #[pyo3(get)]
    pub warnings: Vec<PyObject>,
    /// Exceptions fuer `event.mark_failed(...)` (ProcessingCriticalError o.ae.).
    #[pyo3(get)]
    pub errors: Vec<PyObject>,
}
