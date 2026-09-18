# Logprep Rust-Migration (PyO3)

**Ziel**: Schrittweise Migration der ng/ (async, uvloop) Codebasis nach Rust über PyO3.
Die Applikation muss zu jedem Zeitpunkt weiter ausführbar bleiben.

## Leitprinzipien

- Nicht-alles-auf-einmal: Jede Phase liefert nutzbare Rust-Funktionen, die von Python aus aufrufbar sind
- Python-Brücke: Rust-Module werden als `logprep._rust` (PyO3) importiert
- genutzte externe Bibliotheken in Python werden wenn möglich durch verfügbare rust crates ersetzt
- sind für genutzte externe Bibliotheken in Python keine rust crates verfügbar, wird die Bibliothek in Rust geschrieben
- Python Code soll nur einen dünnen Wrapper darstellen um die API nicht zu brechen. Funktionen werden in Rust implementiert.
- Kein Big Bang: Bestehende Python Implementierungen werden Schritt für Schritt nach Rust migriert
- Keine Python Fallbacks als Alternative für Rust Funktionen
- ng/ als Basis: Alle Rust-Komponenten werden für die ng/ (async) Codebasis entwickelt
- 100% Testabdeckung für Rust und Python Code
- keine Performance Regressions. Nach jeder Phase werden Performance Tests mit der benchmark infrastruktur gemacht (`./benchmarks`)

---

## Phase 1: PyO3-Setup + Alle Dotted-Field Helper

**Ziel**: Rust-Build-Pipeline etablieren, alle 24 Dotted-Field-Funktionen aus `logprep/util/helper.py` nach Rust migrieren. Nach Phase 1 enthält `helper.py` nur noch thin Python-Wrapper und nicht-dotted-field Hilfsfunktionen.

**Begründung**: Die 24 Dotted-Field-Funktionen bilden das Fundament aller Prozessoren. Sie sind rein rechenintensiv, haben keine I/O-Abhängigkeiten und werden von fast jeder Datei genutzt. Eine vollständige Migration in Phase 1 vermeidet gemischte Python/Rust-Logik über mehrere Phasen.

**Wichtig**: Nach dem Wechsel zu maturin existiert **kein Python-Fallback**. Die Rust-Bibliothek ist zwingende Build-Dependency. `try/except ImportError`-Blöcke sind nicht erlaubt.

**Impakt**: 19 rechenintensive Kernfunktionen wandern komplett nach Rust. `helper.py` enthält nur noch thin Python-Wrapper (je 1-3 Zeilen) und nicht-dotted-field Helfer wie `Missing`, `DottedTemplate`, `recursive_compare`, `camel_to_snake`, etc.

**Schritte:**

- **1a — Rust-Scaffolding**: Cargo-Workspace (`Cargo.toml`, `crates/logprep-core/`) anlegen. Rein additiv, kein Bestandscode betroffen. Enthält PyO3-Moduldefinition und leeres `field.rs`. Verifizierung via `cargo build` + `cargo test`, Python bleibt unberührt.

- **1b — maturin als Build-Backend + Nix-Integration**: Python-Build-Backend von `uv_build` auf `maturin` umstellen. PyO3-Modul wird als `logprep._rust` importiert. Nix-Flake um Rust-Toolchain-Input erweitern, `cargo`/`rustc` im Build-Environment verfügbar machen. Performance-Baseline vor Rust-Aktivierung sichern.

- **1c — Parsing + Joining in Rust**: Drei Parser-Funktionen migrieren — das Fundament für alle folgenden. `get_dotted_field_list` (splittet Dotted-Field-String, unterstützt Escaping), `field_list_to_dotted_field` (kombiniert Liste zu Dotted-Field mit Escaping), `join_dotted_fields` (kombiniert ohne Escaping). Der `lru_cache` auf `get_dotted_field_list` entfällt — Rust ist schneller ohne Cache.

- **1d — Read-Operationen in Rust**: Sechs Read-Funktionen migrieren. `_get_item` als interne Helper-Funktion (Dict-Zugriff, List-Index, List-Slice). `get_dotted_field_value` (traversiert Event-Pfad, gibt `None` bei fehlendem Key), `get_dotted_field_value_with_missing` (wie vorher, aber wirft Exception bei fehlendem Key), `get_field_value` (Zugriff über bereits gesplittete Feld-Liste), `get_field_value_no_slice` (optimierte Variante ohne Slice-Unterstützung), `get_dotted_field_values` (Batch-Read mehrerer Felder mit `on_missing`-Callback und SKIP-Unterstützung).

- **1e — `has_dotted_field` in Rust**: Existenz-Check migrieren. Prüft ob ein Dotted-Field im Event existiert, mit `allow_none`-Flag zur Unterscheidung zwischen `None`-Wert und fehlendem Key.

- **1f — Pop-Operationen in Rust**: Drei Pop-Funktionen migrieren. `_pop_field_value` (entfernt Feld auf letzter Ebene und gibt Wert zurück), `_pop_field_value_and_drop_empty` (rekursives Pop, entfernt leere Eltern-Dicts), `pop_dotted_field_value` (exportierte API mit `drop_empty`-Flag).

- **1g — Write-Operationen in Rust**: Fünf Add/Write-Funktionen + `FieldExistsWarning`-Exception migrieren. `_add_and_overwrite_key` (erzeugt/überschreibt Sub-Dict), `_add_and_not_overwrite_key` (erzeugt Sub-Dict nur wenn nicht existent), `_add_field_to` (Kern-Logik mit Merge/Overwrite/Exists-Prüfung), `_add_field_to_silent_fail` (fehlertolerante Variante, sammelt skipped_fields), `add_fields_to` (Batch-Add mehrerer Felder mit None-Filter). Deep-Copy via Python's `copy.deepcopy`.

- **1h — Thin Wrapper + Aufräumen**: Fünf Python-Wrapper auf 1-3 Zeilen reduzieren (`append_as_list` als `partial`, `add_and_overwrite`, `append`, `get_source_fields_dict`, `copy_fields_to_event`). Alle 19 migrierten Funktionen sowie `_get_item`, `_get_slice_arg`, `_add_and_overwrite_key`, `_add_and_not_overwrite_key` werden aus `helper.py` entfernt. Zurück bleiben nur Sentinels (`Missing`, `MISSING`, `Skip`, `SKIP`), `DottedTemplate`, Typ-Aliase, und nicht-dotted-field Helfer.

---

## Phase 2: Filter-Engine (FilterExpression AST + Lucene-Parser)

**Ziel**: Die gesamte Filter-Expression AST (14/15 Klassen) plus den Lucene-Query-Parser komplett in Rust implementieren. Python-Code wird auf Import-Wrapper reduziert. `luqum` (externes Python-Paket) wird durch nativen Rust-Parser ersetzt. Rust-Kern arbeitet ohne PyO3 auf `serde_json::Value`.

**Begründung**: Der Filter-Expression-Matching-Code ist der Hot Path für jedes Rule-Matching im System — er wird für jede Nachricht und jedes Rule aufgerufen. Aktuell nutzt er Python's `re`-Modul für Wildcard/Regex-Matching. Eine Rust-Implementierung eliminiert den Python-Overhead komplett. Die Rust-Klassen sollen zukünftig direkt benutzt werden, daher müssen sie ohne PyO3 funktionieren. Zwischenzeitlich wird eine dünne Schicht benötigt, die Rust-Klassen mit Python-Objekten koppelt.

**Impakt**: Filter-Matching eliminiert Python-Overhead komplett. Rust-Enum-basierte Implementierung arbeitet auf `serde_json::Value` statt Python-Dicts. Rust-Klassen sind unabhängig von PyO3 nutzbar (z.B. für zukünftige Rust-Pipelines). `luqum`-Abhängigkeit entfällt, `regex`-Crate ersetzt `re`-Modul. Zwei-Fakultäts-Architektur: Pure Rust Core (kein PyO3) + dünner PyO3-Adapter für die Python-API.

**Betroffene Dateien**: `logprep/filter/expression/filter_expression.py` (449 Zeilen, 14 Klassen → gelöscht), `logprep/filter/expression/__init__.py` (leer → Import-Wrapper), `logprep/filter/lucene_filter.py` (745 Zeilen, `luqum`-Abhängigkeit → Thin Wrapper). 13 Import-Sites in rule_tree, rule.py, event.py bleiben unverändert.

**Schritte:**

- **2a — Pure Rust Core + PyO3-Adapter (Expression-Typen)**: Alle 15 Expression-Varianten als Rust-Enum (`FilterExpressionInner`) mit `matches()` (safe — gibt `false` bei fehlenden Keys) und `does_match()` (fallibel — wirft `MatchError`). Match-Logik arbeitet auf `serde_json::Value`. PyO3-Adapter (`PyFilterExpression`) als dünne Schicht für Python-API mit `pydict_to_json`-Konverter. Factory-Funktionen für jede Expression-Variante. Hilfsfunktionen für Wildcard-Regex-Erstellung (`*` → `.*`, `?` → `?.`), Sigma-Regex (case-insensitive), und Regex-Normalisierung (`^`/`$`-Anchors). 30+ Rust-Unit-Tests, kein Python-GIL nötig.

- **2b — Lucene-Parser in Rust**: Eigener Lexer + Parser, der Lucene-Query-Strings in `FilterExpressionInner`-Bäume parst. Ersetzt `luqum` vollständig. Unterstützt: `AND`, `OR`, `NOT`-Operatoren, Klammerung, Feld-spezifische Filter (`field:value`), Bereichsabfragen (`[x TO y]`, `{x TO y}`), Regex-Felder (`field:/pattern/`), `null`-Felder, Wildcard-Strings (werden je nach Kontext zu Wildcard/Sigma/Regex-Expressions), Phrasen (in Anführungszeichen), Lucene-Escaping (`remove_lucene_escaping`/`add_lucene_escaping`), `special_fields`-Unterstützung (regex_fields/sigma_fields via Python-Dict). Python-Brücke `parse_lucene_query`.

- **2c — Python-Bridge (Import-Wrapper)**: `expression/__init__.py` wird auf Rust-Re-Exporte umgestellt — alle 15 Factory-Funktionen werden unter denselben Namen wie die alten Klassen bereitgestellt (z.B. `Always`, `And`, `StringFilterExpression`). `lucene_filter.py` wird zum Thin Wrapper: `LuceneFilter.create()` delegiert an Rust's `parse_lucene_query`. Alle bestehenden Import-Sites funktionieren unverändert. API-Änderung: `isinstance(expr, Always)` → `expr.expression_type == "Always"`. `FilterExpression` ist jetzt eine einzelne Rust-Klasse statt Python-Vererbungshierarchie.

- **2d — Aufräumen + `luqum` entfernen**: `filter_expression.py` (449 Zeilen) löschen, alte `lucene_filter.py` durch dünnen Wrapper ersetzen. `luqum` aus `pyproject.toml`-Abhängigkeiten und `mypy`-Modul-Ignorieren entfernen. `uv.lock` neu generieren. Interne Escaping-Tests nach Rust verschieben. 100% Testabdeckung sicherstellen.

---

## Phase 3: Rule Tree + Rule Matching

**Ziel**: `RuleTree` und die gesamte Rule-Parsing-Pipeline (DeMorganResolver, RuleSegmenter, CnfToDnfConverter, RuleSorter, RuleTagger, RuleParser, Node) komplett in Rust implementieren. Python-Code wird auf einen dünnen Wrapper am `RuleTree` reduziert, der Rule-IDs (u64) auf Python-`Rule`-Objekte mapped.

**Begründung**: Zentraler Matching-Mechanismus, der von jedem `Processor.process()` aufgerufen wird. Die Rust-Filter-Engine aus Phase 2 liefert `FilterExpressionInner` — die RuleTree-Komponenten arbeiten ab Phase 3 direkt damit, ohne jemals in Python-Objekte zu konvertieren.

**Leitprinzip**: Innerhalb der migrierten Rust-Komponenten werden **keine Python-Objekte** verwendet. Der gesamte RuleTree arbeitet intern mit `FilterExpressionInner` (Enum aus Phase 2), `serde_json::Value` für Event-Dokumente, und Integer Rule-IDs (`u64`). Der Übergang zu Python geschieht ausschließlich am `RuleTree`-Wrapper.

**Impakt**: Zentraler Matching-Mechanismus läuft komplett in Rust auf `serde_json::Value` — kein Python-Objekt-Overhead mehr für `Node.does_match()`. Jeder `child.does_match(event)`-Aufruf war vorher ein Python-Methodenaufruf, der `KeyDoesNotExistError` abfängt — jetzt ein direkter Rust-Enum-Match. Rule-Parsing (DeMorgan, DNF, Tagging) läuft ohne GIL-Overhead. Datenkonvertierung (Python-Dict → `serde_json::Value`) passiert nur einmal pro Event. Erwartete Durchsatzsteigerung von 5-15%.

**Schritte:**

- **3a — NodeInner + TreeInner (Pure Rust Core)**: Kein PyO3, keine Python-Objekte. `NodeInner` mit Expression (Optional), Children-Vec und `matching_rule_ids` (Vec<u64>). `TreeInner` mit Root-Node, `add_rule()` (segments + rule_id) mit Baum-Traversierung und Child-Sharing, `get_matching_rules()` (DFS-Traversierung, besucht nur matchende Child-Nodes) mit Deduplizierung. `NodeInner::does_match()` ruft direkt `FilterExpressionInner::matches()` auf. 10+ Rust-Unit-Tests.

- **3b — Rule-Parsing-Pipeline in Pure Rust**: Fünf Parser-Komponenten + `RuleParserInner`-Orchestrator, alle auf `FilterExpressionInner`-Enums. Pipeline: DeMorganResolver (löst NOT-Expressions rekursiv auf: NOT(A AND B) → (NOT A) OR (NOT B), doppelte Negation aufheben) → RuleSegmenter/CnfToDnfConverter (segmentiert in DNF: äußere Vec = OR, innere = AND; distributives Gesetz für CNF→DNF) → RuleSorter (sortiert Segmente nach priority_dict, Always zuerst) → AddExistsFilter (fügt vor Key-basierten Ausdrücken Exists-Checks ein, um frühzeitig abbrechen zu können) → RuleTagger (fügt Tag-Checks via tag_map hinzu). 50+ Rust-Unit-Tests. `indexmap` als neue Dependency.

- **3c — PyO3-Adapter (PyRuleTree) + Python Thin Wrapper**: `PyRuleTree` als PyO3-Klasse, die `TreeInner` wrappt. `add_rule(rule_id, segments)`, `get_matching_rules(event)` (konvertiert Python-Dict zu `serde_json::Value` via `pydict_to_json`), `parse_rule(filter_expr, priority_dict, tag_map)` (gibt DNF-Segmente als Python-Liste von PyFilterExpression zurück). Python-`RuleTree` als dünner Wrapper mit `_rule_id_to_rule`-Dict: `add_rule()` → `inner.parse_rule()` + `inner.add_rule()`, `get_matching_rules()` → `inner.get_matching_rules()` + Rule-Lookup. `from_py_object()` zur Extraktion von `FilterExpressionInner` aus Python-Objekten.

- **3d — Alten Python-Code entfernen + Phase-2-Wrapper-Analyse**: 6 Python-Dateien in `logprep/framework/rule_tree/` löschen (`node.py:105 Z.`, `demorgan_resolver.py:70 Z.`, `rule_segmenter.py:268 Z.`, `rule_sorter.py:97 Z.`, `rule_tagger.py:122 Z.`, `rule_parser.py:134 Z.`). Python-Tests werden auf den neuen `RuleTree`-Wrapper umgestellt (testen indirekt über `add_rule` + `get_matching_rules`). Phase-2-Wrapper-Abbau: `LuceneTransformer`, `KeyBasedFilterExpression`, `CompoundFilterExpression`, `RangeBoundary`, `_get_value` können entfallen. Factory-Funktionen, `FilterExpression` (Typ-Alias) und Exceptions müssen erhalten bleiben.

---

## Phase 3.5: Processor-Orchestrierung (Base-ABC) in Rust

**Ziel**: Die gesamte Event-Verarbeitungs-Orchestrierung aus `logprep/ng/abc/processor.py` (`_process_rule_tree`, `_process_rule_tree_multiple_times`, `_apply_rules_wrapper`, `_handle_warning_error`, `_has_missing_values`, `_write_target_field`, `delete_source_fields`-Aufräumen, `apply_multiple_times`-Loop, Filter-Matching, Metrik-Zählung der getroffenen Rules) wird in einen Rust-`ProcessorCore` migriert. **Jeder** Processor in ng/ läuft danach intern über diesen Rust-Kern — auch solche, deren `_apply_rules`-Logik noch in Python steht (via PyO3-Callback). Damit ist die Voraussetzung für Leitanforderung 1 (intern nur noch Rust) bereits nach Phase 3.5 erfüllt, unabhängig vom Fortschritt der per-Prozessor-Migration in Phase 4.

**Begründung**: Phase 3 migriert `RuleTree` + Rule-Parsing. Phase 4 migriert 9 der ~30 Prozessoren. Würde die Orchestrierung bis Phase 4 in Python verbleiben, müssten die Phase-4-Prozessoren den `_process_rule_tree`-Pfad der ABC旁路ieren (`async def process` überschreiben), während die übrigen ~21 Prozessoren weiterhin komplett in Python über die ABC laufen. Es entstünden zwei inkonsistente Ausführungspfade, und die Anforderung „intern nur noch Rust" wäre für un-migrierte Prozessoren verletzt. Phase 3.5 zieht die gemeinsame Orchestrierung vor die per-Prozessor-Migration und schafft damit einen einheitlichen Ausführungspfad: den Rust-`ProcessorCore` mit optionalem Python-Callback für un-migrierte Rules.

**Leitprinzip (verbindlich)**: Rust liefert pro `process()` eine outcome-Struktur mit den **getroffenen Rule-IDs** (`matched_rule_ids`), den aufgetretenen Warnings und Fehlern. Python konsumiert ausschließlich diesen Outcome; **kein** Python-Code ruft nach Phase 3.5 noch `self._rule_tree.get_matching_rules(...)` auf. Der `matched_rule_ids`-Vertrag ist der definierte Seam für die spätere Rust-Metrics-Migration (Anforderung 3): dann übernimmt ein Rust-Metrics-Registry die Zähler, und `Rule.Metrics` wird zum dünnen PyO3-Blick darauf. Bis dahin bleibt `Rule.Metrics` die Source of Truth; Python pflegt die Zähler aus `matched_rule_ids` nach.

### Rust-Modulstruktur

```
crates/logprep-core/src/
├── processor/
│   ├── mod.rs                  # PyO3-Registration, re-exports
│   ├── core.rs                 # ProcessorCore inkl. ProzessOutcome + Callback-Path  (NEU in 3.5)
│   └── outcome.rs              # ProcessOutcome, ProcessingWarning-Rückgabetyp        (NEU in 3.5)
```

### Datenmodell (Rust)

```rust
// crates/logprep-core/src/processor/outcome.rs
#[pyclass]
pub struct ProcessOutcome {
    #[pyo3(get)]
    pub matched_rule_ids: Vec<u64>,       // von TreeInner geliefert
    // bei with_timing=true später: pub timings: HashMap<u64, f64>
    pub warnings: Vec<PyProcessingWarning>,
    pub errors: Vec<PyProcessingError>,
    #[pyo3(get)]
    pub delete_source_fields: Vec<(u64, Vec<String>)>,  // (rule_id, source_fields) wie processor.py:192-196
}

// crates/logprep-core/src/processor/core.rs
pub struct ProcessorCore {
    name: String,
    tree: TreeInner,
    apply_multiple_times: bool,
    rule_id_to_rule: HashMap<u64, PyObject>,   // Python-Rule-Referenz (für Metrik-Inkrement via Outcome)
    rule_specs: HashMap<u64, Box<dyn RuleSpec>>,  // belegt erst in Phase 4 (4b)
}

impl ProcessorCore {
    pub fn add_rule(&mut self, rule_id: u64, filter: &FilterExpressionInner,
                    segments: Vec<Vec<FilterExpressionInner>>,
                    py_rule: PyObject) -> Result<(), String> {
        self.tree.add_rule(rule_id, &segments);
        self.rule_id_to_rule.insert(rule_id, py_rule);
        Ok(())
    }

    pub fn process(&self, py: Python, event: &Bound<'_, PyDict>,
                   apply_hook: Option<PyObject>) -> PyResult<ProcessOutcome> {
        let mut value = pydict_to_json(event)?;
        let mut matched = self.tree.get_matching_rules(&value);
        if self.apply_multiple_times { matched = self.dedup_multiple(&value, matched); }
        let mut outcome = ProcessOutcome::default();
        for rule_id in &matched {
            if let Some(spec) = self.rule_specs.get(rule_id) {
                spec.apply(&mut value).map_err(...)?;           // Phase 4: pure Rust
            } else if let Some(ref hook) = apply_hook {
                hook.call(py, (rule_id, &mut value, ...))?;     // Phase 3.5: Python-Callback
            }
            // delete_source_fields, warning-tag-merge, data_error-skip — in Rust
        }
        json_to_pydict(py, event, &value)?;
        outcome.matched_rule_ids = matched;
        Ok(outcome)
    }
}

#[pyclass]
pub struct PyProcessorCore { inner: ProcessorCore }
```

`RuleSpec`-Trait wird bereits hier deklariert (ohne Implementierungen), damit `rule_specs` existiert — belegt erst in Phase 4 Subphase 4b:
```rust
pub trait RuleSpec: Send + Sync {
    fn apply(&self, event: &mut Value) -> Result<(), String>;
}
```

### Python-Adapter nach Phase 3.5

```python
# logprep/ng/abc/processor.py  —  Orchestrierung delegiert an ProcessorCore
from logprep._rust.processor import PyProcessorCore

class Processor(Component):
    def __init__(self, name, configuration, _rust_processor_factory=None):
        super().__init__(name, configuration)
        self._core = PyProcessorCore(
            name=self.name,
            apply_multiple_times=self.config.apply_multiple_times,
        )
        self._rule_id_to_rule: dict[int, Rule] = {}
        self.load_rules(rules_targets=self.config.rules)
        self._bypass_rule_tree = bool(ENV_VARS.get("LOGPREP_BYPASS_RULE_TREE"))

    def load_rules(self, rules_targets):
        for rule in RuleLoader(rules_targets, self.name).rules:
            self._core.add_rule(rule_id=rule._intern_id, filter=rule.filter, segments=...,
                                  py_rule=rule)
            self._rule_id_to_rule[rule._intern_id] = rule

    async def process(self, event):
        self._event = event
        outcome = self._core.process(event.data, apply_hook=self._apply_rule_in_python)
        # Metrik-Inkrement ausschließlich über outcome.matched_rule_ids (Vertrag für Anforderung 3)
        for rid in outcome.matched_rule_ids:
            self._rule_id_to_rule[rid].metrics.number_of_processed_events += 1
        for warning in outcome.warnings:
            self._event.warnings.append(warning)
        return self._event

    # Callback für noch nicht migrierte Prozessoren. Signatur kompatibel mit RuleSpec::apply.
    def _apply_rule_in_python(self, py_rule_id, py_event, ...):
        rule = self._rule_id_to_rule[py_rule_id]
        self._apply_rules(py_event, rule)
```

### Warum kein旁路 der ABC

In Phase 4试过 der挫败en Variante wurde `async def process` pro Processor überschrieben und der ABC `process`旁路iert. Phase 3.5 macht das unnötig: `process` bleibt in der ABC, ruft `self._core.process(...)`. Phase 4-Subphasen registrieren nur `RuleSpec`-Einträge — der ABC-Pfad bleibt für alle Prozessoren gleich.

### Schritte

- **3.5a — `ProcessorCore` + `ProcessOutcome` in Rust**: Pure-Rust-Orchestrierung inkl. `apply_multiple_times`-Loop (Differenz-Menge wie `processor.py:149-156`), `delete_source_fields`-Aufräumen (`processor.py:192-196`), warning-tag-merge (`_handle_warning_error:233-251`) und `data_error`-Skip (`_apply_rules_wrapper:174-180`). Nutzt `field::value::*` aus Phase 1. PyO3-Adapter `PyProcessorCore` mit `add_rule` + `process(apply_hook)`. Callback-Path via `Option<PyObject>`. 30+ Rust-Unit-Tests.

- **3.5b — Python-ABC auf ProcessorCore umstellen**: `logprep/ng/abc/processor.py` auf Adapter (siehe oben) reduzieren. `_process_rule_tree`, `_process_rule_tree_multiple_times`, `_process_rule`, `_apply_rules_wrapper`, `_handle_warning_error` entfallen als Python-Methoden; ihre Logik lebt in Rust. `_apply_rules` bleibt `@abstractmethod` als Python-Callback-Implementierung. `LOGPREP_BYPASS_RULE_TREE` wird im Rust-Core respektiert.

- **3.5c — Verifizierung über die gesamte Suite**: Alle ~30 Prozessoren laufen unverändert grün, da un-migrierte über den Callback-Pfad ihre `_apply_rules` ausführen. Negativkatalog-Vorprüfung: kein `tree.get_matching_rules` mehr in `ng/processor/*/processor.py`. Performance-Test gegen Phase-3-Baseline; Toleranz < 3% (Rust-Overhead durch Callback-Roundtrip ist temporär).

### Akzeptanzkriterien (Phase 3.5 abgeschlossen)

- [ ] `crates/logprep-core/src/processor/core.rs` mit `ProcessorCore` + `ProcessOutcome`; `rule_specs: HashMap<u64, Box<dyn RuleSpec>>` (leer bis Phase 4).
- [ ] `logprep/ng/abc/processor.py` delegiert `process` vollständig an `PyProcessorCore`; keine `_process_rule_tree`-Methode mehr in Python.
- [ ] Keine `*.py` unter `logprep/ng/processor/` ruft `_rule_tree.get_matching_rules(...)` auf.
- [ ] `matched_rule_ids` ist der einzige Weg, wie Python an Rule-Metriken gelangt — dokumentierter Vertrag (Kommentar + `AGENTS.md`-Eintrag).
- [ ] Alle ~30 Prozessoren: Unit- und Acceptance-Tests grün ohne Modifikation.
- [ ] Performance < 3% Regression ggü. Phase-3-Baseline.

---

## Phase 4: Processor-RuleSpecs (Einfache Processor)

**Ziel**: Die neun einfache, rechenintensive Prozessoren werden — pro Processor durch ein `RuleSpec`-Struct mit `apply(&mut Value)` — in Rust migriert. Schritt 4i erweitert die Phase auf alle verbleibenden 23 ng-Prozessoren (Wellen A–C; I/O-/ML-Prozessoren der Gruppe D bleiben vorerst im Callback-Pfad). Die Orchestrierung (Matching, Warning-Handling, `delete_source_fields`, Metrik-Zählung) liegt seit Phase 3.5 im gemeinsamen `ProcessorCore`; Phase 4 belegt lediglich die `rule_specs`-Tabelle des Cores pro migriertem Processor. Sobald ein `RuleSpec`-Eintrag existiert, entfällt für diesen Processor der Python-Callback-Pfad aus 3.5. **Keine `process()`-Methode eines Phase-4-Prozessors旁路iert die ABC** — die ABC ruft weiterhin `self._core.process(...)`, der Core dispatcht an `RuleSpec::apply`. Die in Phase 1 migrierten Helper (`pop_dotted_field_value`, `add_fields_to`, `get_dotted_field_value`, …) werden in Phase 4 wiederverwendet, nicht neu implementiert. Sämtliche externen Python-Bibliotheken der neun priorisierten Prozessoren (`pyparsing`, `msgspec`, `base64`, Python-`re`, `dataclasses`, `functools.partial`, `functools.cached_property`, `timeout`-Decorator) werden durch Rust-Crates oder direkt in Rust geschriebenen Code ersetzt — **mit Ausnahme von `attrs`**, das an den Python-`Rule`-Klassen für die externe Introspection-API erhalten bleibt (siehe Leitprinzip 3).

**Begründung**:
- Nach Phase 3.5 läuft die Orchestrierung bereits in Rust; der verbleibende Python-Overhead pro Event ist der Callback-Roundtrip für un-migrierte Prozessoren. Phase 4 beseitigt diesen Roundtrip für die neun priorisierten Prozessoren, indem `apply` direkt in Rust läuft.
- Indem die Rule-Datenstrukturen (z. B. `DropperRuleSpec`, `FieldManagerRuleSpec`) in Rust als `serde::Deserialize`-Structs definiert werden, fällt der Python-Validator-Overhead pro `add_rule` weg, und die Validierung kann in `RuleSpec::validate` streng typisiert zwischen `add_rule` und `apply` geteilt werden.
- Externe Bibliotheken wie `pyparsing` (Calculator), `msgspec` (Decoder) und `re` (Dissector/Replacer) ziehen C-Extension-Overhead, globale Locks oder großen Speicherbedarf nach sich. Rust-Äquivalente (eigener Pratt-Parser, `serde_json`, `regex`-Crate) sind bereits als Crates verfügbar oder werden in 4g direkt geschrieben.
- Da die ABC (Phase 3.5) als einheitlicher Ausführungspfad erhalten bleibt, ist die bestehende `ng.abc.processor.Processor`-Schnittstelle (mit `setup`, `metrics`, `process(event) -> LogEvent`) für alle Prozessoren identisch — unabhängig vom Migrationsstand.

### Leitprinzipien (verbindlich)

1. **Keine Event-Verarbeitung in Python**: Weder `logprep/ng/processor/<name>/processor.py` noch `logprep/ng/abc/processor.py` iterieren über Event-Felder, rufen `pop_dotted_field_value`/`add_fields_to`/`get_dotted_field_value` zur Rule-Anwendung auf oder werten `for rule in matching_rules` aus. Die ABC-`process`-Methode (Phase 3.5) ruft `self._core.process(...)`; migrierte Prozessoren stellen keinen `_apply_rules`-Callback mehr bereit (das `rule_specs`-Slot trifft). `_apply_rules` entfällt pro migriertem Processor vollständig.

2. **rule_tree nur via Rust**: `tree.get_matching_rules(...)` wird ausschließlich innerhalb des Rust-`ProcessorCore` (Phase 3.5) aufgerufen. Python-Code sieht nur das `ProcessOutcome` (`matched_rule_ids` + Warnings).

3. **`RuleSpec` in Rust, `Rule`-Python-API bleibt erhalten**: Die processor-spezifische Rule-Logik wird in Rust als `RuleSpec`-Struct mit `apply(&self, event: &mut Value)` + `validate(&Map) -> Result<(), String>` definiert und beim `add_rule` in den `rule_specs`-Slot des Cores eingetragen. **ABER**: die Python-`Rule`-Klasse (`logprep/processor/<name>/rule.py`) bleibt als `attrs`-`@define` mit voller `Config` bestehen — sie ist die externe Introspection-API (`rule.drop`, `rule.source_fields`, `rule.description`, `rule.filter_str`, `rule.metrics`, `rule_class`-Attribut, `rules`-Property). Beim `add_rule` wird die Python-Rule *und* der Rust-`RuleSpec` parallel registriert: die Python-Seite für externe Introspection/Metriken-Source-of-Truth, die Rust-Seite für `apply`. `attrs` wird **nicht** aus den Python-Rule-Klassen entfernt; nur die pro-Event-Verarbeitung wandert nach Rust.

4. **Externe Python-Bibliotheken → Rust-Crate oder Rust-Implementierung**: Die Tabelle in [§4.0 Crate-Entscheidungen](#40-crate-entscheidungen) ist verbindlich. Keine Phase-4-Implementierung darf eine dort nicht aufgeführte externe Python-Bibliothek für die Event-Verarbeitung einführen. `attrs` an Python-Rule-Klassen ist von diesem Verbot ausgenommen (Leitprinzip 3).

5. **Phase-1-Helper werden wiederverwendet**: Die in Phase 1 nach Rust portierten Funktionen (`pop_dotted_field_value`, `pop_field_value`, `add_field_to`, `add_field_to_silent_fail`, `add_and_overwrite_key`, `add_and_not_overwrite_key`, `get_dotted_field_value`, `get_dotted_field_value_with_missing`, `get_dotted_field_values`, `has_dotted_field`, `get_source_fields_dict`, `resolve_template`, `add_and_overwrite`, `append`, `append_as_list`, `get_dotted_field_list`, `field_list_to_dotted_field`, `join_dotted_fields`) werden in Phase 4 über `pub(crate) fn` aus `crates/logprep-core/src/field/value.rs` konsumiert (siehe Subphase 4a). **Diese Funktionen werden in Phase 4 nicht reimplementiert.** Deduplizierung wird durch direkten Funktionsaufruf sichergestellt.

6. **Kein Python-Fallback**: Nach Phase 4 gibt es für `pop_dotted_field_value` etc. weiterhin den PyO3-Wrapper (in `crates/logprep-core/src/field/py.rs`) — dieser ist ein **dünner Adapter**, der `field::value::pop_dotted_field_value` aufruft, nicht umgekehrt. Wer in `process()` / `_apply_rules` einer ng-Processor-Klasse noch `pop_dotted_field_value` direkt aufruft, signalisiert, dass die Migration des Prozessors unvollständig ist.

7. **Metriken: Source of Truth bleibt Python, Rust liefert `matched_rule_ids`**: Rust liefert pro `process()` die `matched_rule_ids` (Phase 3.5 Outcome). Python inkrementiert `Rule.metrics.number_of_processed_events` ausschließlich über diese IDs:
   ```python
   for rid in outcome.matched_rule_ids:
       self._rule_id_to_rule[rid].metrics.number_of_processed_events += 1
   ```
   Es wird **niemals** über alle Rules iteriert (`self._rule_id_to_rule.values()`), da nur die getroffenen Rules zählen. `matched_rule_ids` ist der definierte Seam für die spätere Rust-Metrics-Migration (Anforderung 3): dann übernimmt ein Rust-Registry die Zähler, und `Rule.metrics` wird zum dünnen PyO3-Blick darauf. Vorher ist **keine** Metrik-Logik in `RuleSpec::apply` zu implementieren.

8. **Filter-Parsing bleibt Python-seitig**: `LuceneFilter.create(...)` aus Phase 2 liefert eine `FilterExpression` (Rust-Klasse) zurück. Die `add_rule`-Pipeline registriert den Filter via `FilterExpressionInner::from_py_object` beim `ProcessorCore`, der ihn über den bereits existierenden `RuleParserInner` (Phase 3) in Segmente parst und dem internen `TreeInner` hinzufügt. Der Python-`RuleLoader` bleibt für File-I/O zuständig.

### Architektur (Ist vs. Soll)

**Ist (nach Phase 3.5, mit un-migriertem Processor)**:
```
Python: Processor.process(event)                      # ABC (Phase 3.5)
  → PyProcessorCore.process(event, apply_hook=…)      # Rust (Phase 3.5)
    ↳ TreeInner::get_matching_rules                   # Rust (Phase 3)
    ↳ für jede rule_id: apply_hook(py, rule_id, ..)   # Callback → Python
      → Python _apply_rules(event, rule)             # Python-Subclass
        → pop_dotted_field_value / add_fields_to / .. # Python-Helper → Rust
        → pyparsing / msgspec / re / base64           # externe Libs
  ← outcome.matched_rule_ids → Python-Metriken
```

**Soll (nach Phase 4, migrierter Processor)**:
```
Python: Processor.process(event)                      # ABC unverändert (Phase 3.5)
  → PyProcessorCore.process(event, apply_hook=None)  # Rust, kein Callback nötig
    ↳ TreeInner::get_matching_rules                  # Rust (Phase 3)
    ↳ für jede rule_id: rule_specs[id].apply(event)  # Rust (Phase 4)
    ↳ field::value::pop_dotted_field_value           # Rust (Phase 1, wiederverwendet)
    ↳ field::value::add_field_to                     # Rust (Phase 1, wiederverwendet)
    ↳ pratt-parser / serde_json / regex               # Crates statt Python-Libs
    ↳ Event als serde_json::Value mutiert            # 0 Python-Roundtrips pro Rule
  ← outcome.matched_rule_ids → Python-Metriken (Source of Truth)
```

### 4.0 Crate-Entscheidungen

| Externe Python-Bibliothek | Verwendet in | Rust-Crate / Rust-Implementierung | Crate-Status |
|---|---|---|---|
| `pyparsing` (BNF-Parser) | `calculator/processor.py`, `calculator/fourFn.py` | **Eigener Pratt-Parser** in `crates/logprep-core/src/processor/calculator/expression.rs` (kein neues Crate, um Dependency-Footprint klein zu halten) | in Rust zu schreiben |
| `msgspec.json.Decoder` | `decoder/decoders.py` (`parse_json`, `parse_docker`) | `serde_json` (bereits als `serde_json.workspace = "1"` in `Cargo.toml`) | ✅ bereits vorhanden |
| `base64`, `binascii` | `decoder/decoders.py` (`parse_base64`) | `base64 = "0.22"` Crate | ✅ hinzufügen |
| Python `re` | `dissector/rule.py`, `replacer/rule.py`, `decoder/decoders.py` (alle Regex-Operationen) | `regex` Crate (bereits als `regex.workspace = "1"` in `Cargo.toml`) | ✅ bereits vorhanden |
| `attrs` (`@define`, `validators`) | Python-Rule-Klassen (externer Zugriff, Introspection, Metriken-Labels) | **bleibt** an `logprep/processor/<name>/rule.py` — Teil der externen Python-API (Leitprinzip 3). Die *Event-Verarbeitungs*-Logik nutzt stattdessen Rust-`RuleSpec` mit `serde::Deserialize` | keine Migration |
| `attrs` (`@define`, `validators`) | Rust-`RuleSpec`-Structs (Event-Verarbeitung) | `serde::Deserialize` mit `#[serde(deny_unknown_fields)]` + pro-Feld-`#[serde(default = "...")]` | natives Rust-Pattern |
| `dataclasses.dataclass` | `replacer/rule.py`, `dissector/rule.py` | Native Rust-Structs | nativ |
| `functools.partial` | `decoder/decoders.py` (`partial(_parse, regexes=...)`) | Rust-Closures (`.map_err(...)`, `move |x| ...`) | nativ |
| `functools.cached_property` | `calculator/processor.py` (BNF als Singleton) | `OnceCell` / `OnceLock` aus `std::sync` | nativ |
| `logprep.util.decorators.timeout` | `calculator/processor.py` | `std::sync::mpsc::channel` + `std::thread::spawn` + `recv_timeout` | nativ |
| `logprep.util.helper` (Python-Wrapper) | Alle `processor.py` | `field::value::*` (pure Rust, wiederverwendet) | bereits in Phase 1 migriert |
| `logprep.processor.calculator.fourFn.BNF` | `calculator/processor.py` | Pratt-Parser in Rust (gleicher Funktionsumfang) | in Rust zu schreiben |
| `hashlib` (SHA256) | `deduplicator`, `pseudonymizer` (`logprep/util/hasher.py`) | `sha2` Crate (Identität zu `hashlib.sha256().hexdigest()` testen) | ✅ hinzufügen |
| `Crypto.Cipher.AES` (GCM) + `RSA/OAEP` | `pseudonymizer` (`logprep/util/pseudo/`) | `aes-gcm` + `rsa` Crates | ✅ hinzufügen |
| `datetime`/`zoneinfo`/`time` Parsing | `timestamper`, `timestamp_differ`, `datetime_extractor` | `chrono` Crate | ✅ hinzufügen |
| `uuid.uuid4` | `pre_detector` | `uuid` Crate (`v4` Feature) | ✅ hinzufügen |
| Python `ipaddress` | `network_comparison`, `ip_informer`, `domain_label_extractor` | `ipnetwork` Crate bzw. eigene Parser (Semantik-Parität zu CPython testen) | teilweise in Rust zu schreiben |
| Grok-Pattern-Engine (`re` + Pattern-Zips) | `grokker` | Eigener Grok-Parser in `processor/grokker/grok.rs` über `regex` Crate; Pattern-Inhalt kommt via Python-Getter beim `add_rule` | in Rust zu schreiben |
| `sklearn`/`joblib` (ML-Inferenz) | `amides` | **keine Migration** — Gruppe D (4i), bleibt Python-Callback; ggf. später ONNX/tract | Ausnahme (dokumentiert) |
| `requests`/`socket` (Verarbeitungszeit-I/O) | `requester`, `domain_resolver`, `generic_resolver`, `geoip_enricher` | **keine Migration** — Gruppe D (4i), I/O bleibt Python; `geoip2`-Lookup ist Kandidat für `maxminddb` Crate sobald Datei-Handling geklärt ist | Ausnahme (dokumentiert) |

**Verboten in Phase 4**:
- `pyo3-asyncio`, `pyo3-asyncio`-basierte tokio-Integration (Phase 6)
- Neue Python-Crates in `pyproject.toml` für Phase-4-Funktionalität
- Jegliche `try: import msgspec; ...; except ImportError: ...`-Konstrukte (Leitprinzip 6)

### Rust-Modulstruktur

```
crates/logprep-core/src/
├── field/
│   ├── mod.rs                # re-exportiert value::* und py::*
│   ├── value.rs              # pure-Rust Operationen auf serde_json::Value  (NEU in 4a)
│   └── py.rs                 # PyO3-Wrapper (alter field.rs-Inhalt)          (UMZUG in 4a)
├── processor/
│   ├── mod.rs                # RuleSpec-Trait, Registration                   (Core aus 3.5)
│   ├── core.rs               # ProcessorCore + ProcessOutcome                 (Phase 3.5)
│   ├── outcome.rs            # ProcessOutcome-Typen                            (Phase 3.5)
│   ├── spec_helper.rs        # gemeinsame Rule-Validierung, SplitFields       (NEU in 4b)
│   ├── dropper.rs            # DropperRuleSpec + PyDropperSpecFactory
│   ├── deleter.rs
│   ├── field_manager.rs      # inkl. Concatenator
│   ├── string_splitter.rs
│   ├── calculator.rs         # inkl. expression.rs (Pratt-Parser)
│   ├── dissector.rs          # inkl. dissect_parser.rs
│   ├── replacer.rs           # inkl. template.rs
│   └── decoder.rs            # inkl. decoders.rs
```

### Datenmodell (Rust)

Die Orchestrierung (`ProcessorCore`, `ProcessOutcome`) existiert seit Phase 3.5. Phase 4 definiert pro Processor nur noch zwei Typen — den `RuleSpec` und die Slot-Registrierung:

```rust
// 1) Pure Rust RuleSpec — keine PyO3, keine Python-Objekte
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DropperRuleSpec {
    pub drop: Vec<String>,
    #[serde(default = "default_true")]
    pub drop_full: bool,
}

impl RuleSpec for DropperRuleSpec {
    const TYPE_NAME: &'static str = "dropper";
    fn validate(_raw: &serde_json::Map<String, Value>) -> Result<(), String> { Ok(()) }
    fn apply(&self, event: &mut Value) -> Result<(), String> {
        for dotted in &self.drop {
            let key = crate::field::value::get_dotted_field_list(dotted);
            crate::field::value::pop_dotted_field_value(event, &key, self.drop_full);
        }
        Ok(())
    }
}

// 2) Registrierung am bestehenden ProcessorCore (Phase 3.5) — kein eigener Processor-Struct
// Beim add_rule eines Phase-4-Prozessors:
pub fn register_dropper_spec(core: &mut PyProcessorCore, rule_id: u64,
                             raw: &serde_json::Map<String, Value>) -> Result<(), String> {
    let spec: DropperRuleSpec = serde_json::from_value(Value::Object(raw.clone()))
        .map_err(|e| e.to_string())?;
    DropperRuleSpec::validate(raw)?;
    core.set_rule_spec(rule_id, Box::new(spec));   // belegt den rule_specs-Slot → apply_hook entfällt
    Ok(())
}
```

`PyProcessorCore::set_rule_spec(rule_id, Box<dyn RuleSpec>)` ist die einzige neue Methode, die Phase 4 an der Core-API ergänzt. Der `process`-Pfad des Cores (Phase 3.5) dispatcht: trifft der `rule_specs`-Slot, läuft `apply` in Rust; sonst der Python-Callback.

```rust
// 3) PyO3-Adapter — pro Processor nur die Spec-Factory, kein eigener Processor-Struct
#[pyclass(name = "DropperSpecFactory")]
pub struct PyDropperSpecFactory;

#[pymethods]
impl PyDropperSpecFactory {
    /// Wird vom Python-Adapter in load_rules pro Rule einmal gerufen.
    fn make_and_register(&self, py: Python, core: &mut PyProcessorCore,
                        rule_id: u64, rule_data: &Bound<'_, PyDict>) -> PyResult<()> {
        let raw = pydict_to_map(rule_data)?;
        register_dropper_spec(core, rule_id, &raw)
            .map_err(pyo3::exceptions::PyValueError::new_err)
    }
}
```

### Python-Adapter (Soll) — exakte Form

```python
# logprep/ng/processor/dropper/processor.py
from logprep._rust.processor import PyDropperSpecFactory as _SpecFactory
from logprep.ng.abc.processor import Processor
from logprep.processor.dropper.rule import DropperRule   # attrs-Config bleibt (externe API)
from logprep.util.rule_loader import RuleLoader


class Dropper(Processor):
    """Drop log events. Event-Verarbeitung vollständig in Rust (Phase 4 RuleSpec)."""

    rule_class = DropperRule   # bleibt — externe Introspection-API

    def __init__(self, name: str, configuration: "Processor.Config") -> None:
        # ABC baut den gemeinsamen ProcessorCore (Phase 3.5) auf.
        Processor.__init__(self, name, configuration)
        self._spec_factory = _SpecFactory()
        self.load_rules(rules_targets=self.config.rules)

    def load_rules(self, rules_targets) -> None:
        # ABC.load_rules kümmert sich um Core-Registrierung (Filter + py_rule);
        # hier zusätzlich den Rust-RuleSpec-Slot belegen, damit apply_hook entfällt.
        super().load_rules(rules_targets)   # registriert im Core via 3.5-Pipeline
        for rule in self._rule_id_to_rule.values():
            self._spec_factory.make_and_register(
                core=self._core,
                rule_id=rule._intern_id,
                rule_data=rule._config.asdict(),
            )

    # Kein _apply_rules mehr — der Rust-RuleSpec übernimmt apply.
    # process() wird von der ABC (Phase 3.5) geerbt und nicht überschrieben.
```

`process` wird **nicht** pro Processor überschrieben — geerbt von der ABC (Phase 3.5), die `self._core.process(...)` ruft und Metriken via `outcome.matched_rule_ids` pflegt. `_apply_rules` entfällt für migrierte Prozessoren. Die Python-`DropperRule`-Klasse (`logprep/processor/dropper/rule.py`) bleibt unverändert eine `attrs`-`@define` mit `Config` und `Metrics` — Quelle für `rule.drop`, `rule.metrics` etc.

> **Negativkatalog**: Diese Imports/Operationen sind im Python-Adapter eines migrierten ng-Prozessors **verboten**:
> - `from logprep.util.helper import pop_dotted_field_value, add_fields_to, get_dotted_field_value, …`
> - `from pyparsing import …`
> - `import msgspec` (zu Decoder-Zwecken — Decoder ist in Rust)
> - `import re` (für Rule-Logik — Regex ist in Rust)
> - `import base64` (Decoder ist in Rust)
> - `self._rule_tree.get_matching_rules(...)`
> - `def _apply_rules(self, event, rule): ...` (migrierte Prozessoren überschreiben dies nicht)
> - `async def process(self, event): ...` (geerbt von ABC; Überschreiben旁passst den Rust-Pfad)
> - `if/elif`-Logik auf Event-Feldern in `load_rules`
>
> Die einzigen Schleifen in Python-Adaptern sind: (a) `for rule in RuleLoader(…).rules` / `super().load_rules` in `load_rules`, (b) `for rule in self._rule_id_to_rule.values()` in `load_rules` zur Spec-Registrierung, und (c) der geerbte Metrik-Loop in der ABC über `outcome.matched_rule_ids`. Alle liegen **außerhalb** des Event-Verarbeitungs-Pfads.

### Schritte

- **4a — Pure-Rust-Helper-Modul `field::value` (Refactor Phase 1)**: Aufteilung von `crates/logprep-core/src/field.rs` in:
  - `field/value.rs` (neu): pure-Rust-Implementierungen aller Phase-1-Funktionen operieren auf `&mut serde_json::Value` / `&serde_json::Value`. Funktions-Signaturen:
    ```rust
    pub fn get_dotted_field_value<'a>(event: &'a Value, key: &[String]) -> Option<&'a Value>;
    pub fn get_dotted_field_value_with_missing<'a>(event: &'a Value, key: &[String]) -> Result<&'a Value, MissingFieldError>;
    pub fn get_dotted_field_values<'a>(event: &'a Value, keys: &[Vec<String>], on_missing: OnMissing) -> Vec<Option<&'a Value>>;
    pub fn has_dotted_field(event: &Value, key: &[String], allow_none: bool) -> bool;
    pub fn pop_dotted_field_value(event: &mut Value, key: &[String], drop_empty: bool) -> Option<Value>;
    pub fn add_field_to(event: &mut Value, field_name: &[String], content: Value,
                        merge_with_target: bool, overwrite_target: bool) -> Result<(), FieldExistsError>;
    pub fn add_fields_to(event: &mut Value, fields: BTreeMap<Vec<String>, Value>,
                         merge_with_target: bool, overwrite_target: bool) -> Result<Vec<String>, Vec<String>>;
    pub fn get_source_fields_dict<'a>(event: &'a Value, source_fields: &[Vec<String>]) -> BTreeMap<String, Option<&'a Value>>;
    pub fn resolve_template(template: &str, source_field_dict: &BTreeMap<String, Value>) -> String;
    pub fn append_as_list<'a>(event: &'a mut Value, fields: BTreeMap<Vec<String>, Value>) -> Result<(), FieldExistsError>;
    pub fn add_and_overwrite(event: &mut Value, fields: BTreeMap<String, Value>) -> Result<(), FieldExistsError>;
    pub fn append(event: &mut Value, field_name: &[String], value: Value) -> Result<(), FieldExistsError>;
    pub fn get_dotted_field_list(dotted: &str) -> Vec<String>;
    pub fn field_list_to_dotted_field(list: &[String]) -> String;
    pub fn join_dotted_fields(list: &[String]) -> String;
    ```
  - `field/py.rs`: bestehende PyO3-Funktionen aus `field.rs` werden zu **1–3-Zeilen-Wrappern** reduziert, die `pydict_to_json` → `value::*` → `json_to_pydict` aufrufen. Public-API in Python (`from logprep._rust import pop_dotted_field_value`) bleibt binärkompatibel.
  - `field/mod.rs`: `pub mod value; pub mod py;` plus `pub use value::*;` für crate-internen Zugriff.
  - 30+ Rust-Unit-Tests, die die Byte-Äquivalenz der pure-Rust-Versionen zu den Python-Versionen verifizieren (Behavior-Preservation-Tests, abgeleitet aus den Phase-1-Tests). **Keine** Tests, die die PyO3-Wrapper direkt prüfen — sie sind jetzt trivial.
  - Cargo.toml: `serde_json` ist bereits vorhanden; **kein neues Crate nötig** für 4a.

- **4b — `RuleSpec`-Trait + Slot-Registrierung (Pure Rust Core)**: Der `RuleSpec`-Trait ist in Phase 3.5 als leerer Trait deklariert. Phase 4b füllt ihn in `crates/logprep-core/src/processor/mod.rs`:
  ```rust
  pub trait RuleSpec: Send + Sync {
      const TYPE_NAME: &'static str;
      fn validate(raw: &serde_json::Map<String, Value>) -> Result<(), String>;
      fn apply(&self, event: &mut Value) -> Result<(), String>;
  }
  ```
  neue Core-Methode `PyProcessorCore::set_rule_spec(&mut self, rule_id: u64, spec: Box<RuleSpec>)` (Phase 3.5 hatte `rule_specs: HashMap<u64, Box<dyn RuleSpec>>` leer vorgesehen). Im `process`-Pfad des Cores (Phase 3.5) gilt danach: trifft der `rule_specs`-Slot → `spec.apply(event)` in Rust; andernfalls Python-Callback (un-migrierte Prozessoren).
  Gemeinsame Helfer in `processor/spec_helper.rs`:
  - `SplitFields::split(raw: &Map) -> Vec<Vec<String>>` — splittet Dotted-Field-Strings einmalig beim `add_rule`. Verwendet `field::value::get_dotted_field_list`.
  - `validate_required_keys(raw, &["drop"])` — prüft Pflichtschlüssel (zusätzlich, nicht ersetzend — der Python-Rule hat ihre eigenen `attrs`-Validatoren, die als Sicherheitsnetz für die externe API erhalten bleiben).
  - `processor::register(m)` exportiert alle Spec-Factory-PyO3-Klassen unter `logprep._rust.processor.<name>_spec` (statt `PyDropper`-Processor-Klassen).
  - 10+ Rust-Unit-Tests für Trait + Helper.

- **4c — Dropper in Rust (Referenz-Implementierung)**: `crates/logprep-core/src/processor/dropper.rs`:
  ```rust
  #[derive(Debug, Clone, serde::Deserialize)]
  #[serde(deny_unknown_fields)]
  pub struct DropperRuleSpec {
      pub drop: Vec<String>,
      #[serde(default = "default_true")]
      pub drop_full: bool,
  }
  impl RuleSpec for DropperRuleSpec {
      const TYPE_NAME: &'static str = "dropper";
      fn validate(_raw: &Map) -> Result<(), String> { Ok(()) }
      fn apply(&self, event: &mut Value) -> Result<(), String> {
          for dotted in &self.drop {
              let key = crate::field::value::get_dotted_field_list(dotted);
              crate::field::value::pop_dotted_field_value(event, &key, self.drop_full);
          }
          Ok(())
      }
  }
  // ... PyDropperSpecFactory wie im Datenmodell-Abschnitt ...
  ```
  PyO3-Adapter `PyDropperSpecFactory` mit `make_and_register(core, rule_id, rule_data)`. 30+ Rust-Unit-Tests inkl. Edge-Cases (escaped Dots, leere Dicts, `drop_full=false`, `drop=[]`). Python `logprep/ng/processor/dropper/processor.py` registriert den Spec in `load_rules` (kein `_apply_rules` mehr). `logprep/processor/dropper/rule.py` (attrs-`DropperRule`) bleibt unverändert für die externe API. Bestehende Dropper-Tests müssen ohne Änderung grün sein.

- **4d — Deleter in Rust**: `DeleterRuleSpec { delete: bool }`. `apply`: wenn `delete`, `*event = Value::Object(Default::default())`. 20+ Rust-Unit-Tests.

- **4e — FieldManager + Concatenator in Rust**: `FieldManagerRuleSpec { source_fields: Vec<String>, target_field: String, mapping: BTreeMap<String, String>, delete_source_fields: bool, overwrite_target: bool, merge_with_target: bool, ignore_missing_fields: bool }`. `apply` verwendet ausschließlich `field::value::{get_dotted_field_value, pop_dotted_field_value, add_field_to, add_field_to_silent_fail}` (Phase-1-Wiederverwendung). `ConcatenatorRuleSpec` erbt von `FieldManagerRuleSpec` und fügt `separator: String` hinzu; `apply` ruft `field_manager.apply` auf und joint die String-Werte mit `separator`. 50+ Rust-Unit-Tests.

- **4f — StringSplitter + Replacer + Decoder in Rust**:
  - `StringSplitterRuleSpec { source_fields, target_field, delimiter, drop_empty }` — `str::split` direkt in Rust.
  - `ReplacerRuleSpec { mapping: BTreeMap<source, target>, templates: Vec<ReplacementTemplate>, ignore_missing_fields, overwrite_target }` — Replacer-Template-Logik (1:1 aus `replacer/rule.py`) in Rust portiert. Regex via `regex` Crate (Phase 1).
  - `DecoderRuleSpec { source_fields, target_field, source_format: DecoderFormat, ignore_missing_fields, overwrite_target, merge_with_target }` mit `DecoderFormat` als Rust-Enum (`Json`, `Base64`, `Clf`, `Nginx`, `SyslogRfc3164`, `SyslogRfc3164Local`, `SyslogRfc5424`, `Logfmt`, `Cri`, `Docker`, `Decolorize`). Implementierung als `match` über `source_format`. JSON via `serde_json` (Phase 1). Base64 via `base64` Crate. Regex via `regex` Crate. **Keine** `msgspec`-, `binascii`- oder Python-`re`-Importe.
  - 60+ Rust-Unit-Tests gesamt.

- **4g — Calculator in Rust (inkl. eigenem Pratt-Parser)**:
  - `crates/logprep-core/src/processor/calculator/expression.rs`: Pratt-Parser mit gleicher Semantik wie `calculator/fourFn.py` (Operatoren `+ - * / ^ > < >= <= == !=`, Funktionen `sin cos tan exp abs trunc from_hex round sgn multiply hypot all`, Konstanten `PI E`, unäres Minus). Ausgabe ist ein `Expr`-AST (Rust-Enum). Evaluator traversiert den AST. **Kein** `pyparsing`.
  - `CalculatorRuleSpec { source_fields, target_field, calc: String, timeout: f64, ignore_missing_fields, overwrite_target, merge_with_target }`.
  - `apply`: (1) `get_source_fields_dict` (Phase 1), (2) `resolve_template(calc, source_field_dict)` (Phase 1), (3) `parse_expression(calc_with_values) -> Expr`, (4) `evaluate_with_timeout(expr, timeout)` via `std::sync::mpsc::channel` + `std::thread::spawn` + `recv_timeout` (ersetzt `@timeout`-Decorator), (5) `add_field_to(target_field, result)` (Phase 1).
  - 40+ Rust-Unit-Tests. **Insbesondere**: Whitespace-Toleranz, Operator-Precedence (`2^3^2 == 2^(3^2)`), unäres Minus, `from_hex`, `round`, `sgn`, `all`, Parse-Fehler-Mapping auf `ProcessingWarning`.

- **4h — Dissector in Rust**:
  - `crates/logprep-core/src/processor/dissector/dissect_parser.rs`: Parser für Dissect-Pattern (Sequenz von `%{key}`, `%{&key}`, `%{?key}`, `%{}`, Separatoren) — übersetzt in `Vec<DissectAction>`. Regex-Spezialfälle (z. B. `%{integer}`, `%{data}`) via `regex` Crate.
  - `DissectorRuleSpec { actions_by_source_field, convert_actions: Vec<(target, Converter)>, ignore_missing_fields }` mit `Converter` als Rust-Enum (`Int`, `Float`, `String`, `Bytes`, `Ip`).
  - `apply` führt die Aktionen via `field::value::*` aus (Phase 1). Convert-Actions nutzen `str::parse::<i64>()` / `::parse::<f64>()`.
  - 40+ Rust-Unit-Tests.

- **4i — Verbleibende Prozessoren (Welle 2)**: Zusätzlich zu den neun priorisierten Prozessoren werden die restlichen 23 ng-Prozessoren migriert. Sie werden nach ihren externen Abhängigkeiten in vier Gruppen mit identischem Regel-Slot-Mechanismus (4b) abgearbeitet. I/O- und ML-abhängige Prozessoren bleiben **bewusst** im Python-Callback-Pfad; ihre Konvertierungs-/Lookup-Logik wandert erst, wenn ein Rust-Äquivalent ohne Regression verfügbar ist (Sequenz: erst Rule-Logik, dann I/O über `OutputSpec`/Connectors):
  - **Gruppe A — reine Event-Logik, ohne `OutputSpec` (15)**: `key_checker` (Feld-Existenz-Check), `generic_adder` (statische `add_fields_to`), `field_name_replacer` (`transform_field_value` + Kollisions-Handler aus `helper.py` → `field::value::transform_field_value`), `selective_extractor`, `labeler` (Tag-Merge), `deduplicator` (Hash-Vergleich; `hashlib` → `sha2`/`blake3` Crate), `template_replacer` (`string.Template` → eigener Placeholder-Parser + `regex`), `datetime_extractor`, `timestamper`, `timestamp_differ` (`datetime`/`zoneinfo` → `chrono` Crate), `network_comparison`, `ip_informer` (Python-`ipaddress` → `ipnetwork` Crate bzw. eigene IPv4/IPv6-Parity zu CPython), `domain_label_extractor` (Public-Suffix-Logik 1:1 aus `logprep/util/url` nach Rust), `list_comparison` (Vergleichslogik in Rust; Listen-Inhalte werden beim `add_rule` via `RuleLoader`/Getter aus Python übernommen — Date-I/O bleibt Python, Leitprinzip 8), `grokker` (eigener Grok-Engine in `processor/grokker/grok.rs` über dem `regex` Crate inkl. rekursiver Pattern-Auflösung; Pattern-Dateien kommen via Getter aus Python).
  - **Gruppe B — Zusatz-Events via `OutputSpec` (2)**: `pre_detector` (inkl. `IPAlerter`; `uuid4` → `uuid` Crate) und `pseudonymizer` (Krypto aus `logprep.util.pseudo`: `Crypto.Cipher.AES` GCM + `RSA/OAEP` → `aes-gcm` + `rsa` Crates, `SHA256Hasher` → `sha2` Crate, URL-Aufspaltung in Rust). Vorher Erweiterung von `ProcessOutcome` um `extra_events: Vec<Value>` (bzw. ein Rust-`PendingOutputs`-Kanal) und eines `OutputSpec`-Pfads im Core, damit RuleSpecs zusätzliche Events emittieren können, ohne die ABC zu umgehen. Metrik-/Outcome-Vertrag bleibt: Python konsumiert `matched_rule_ids` + `extra_events`.
  - **Gruppe C — große, aber pure Logik (1)**: `clusterer` (Signature-Phases/Distanz-Metriken komplett in Rust; Portierung der Legacy-Tests als Rust-Unit-Tests, da das Regelwerk sehr fehleranfällig ist).
  - **Gruppe D — bleibt vorerst im Callback-Pfad (5, Ausnahme dokumentieren)**: `requester` (`requests`-HTTP zur Verarbeitungszeit), `domain_resolver` (`socket`-DNS), `generic_resolver` (HTTP-basierter Datei-Cache), `geoip_enricher` (`geoip2`-MMDB — Candidate für `maxminddb` Crate, aber Datei-/filelock-Handling läuft zur Verarbeitungszeit), `amides` (sklearn/joblib-ML-Inferenz ohne tragfähiges Rust-Äquivalent; ggf. später ONNX-basiert). Für diese fünf bleibt `_apply_rules` als Python-Callback aktiv; sie sind die treibende Kraft hinter Phase 5.5 (Rückbau des Roundtrip-Overheads), da ihr Roundtrip-Pfad bis zu einer späteren Phase bestehen bleibt.
  - Nach jeder Untergruppe: Unit-Tests des jeweiligen Prozessors ohne Änderung grün, Eintrag in `scripts/PHASE4_DONE.txt`, Spec-Factory unter `logprep._rust.processor.<name>_spec` registrieren, Benchmark gegen Phase-3.5-Baseline.

- **4j — Aufräumen + Legacy-Path-Validierung + Negativkatalog-Check**:
  1. `logprep/processor/<name>/processor.py` wird zum **Re-Export** (für nicht-ng-Pfad), der `logprep.ng.processor.<name>.processor` importiert. Damit existiert nur noch **eine** Logik-Quelle (Rust via Core) und der nicht-ng-Pfad ist nur ein dünner Python-Alias. **`logprep/processor/<name>/rule.py` bleibt unverändert** — die `attrs`-`Rule`-Klasse ist die externe Introspection-API und Source der Metriken; sie wird *nicht* re-exportiert oder reduziert.
  2. `logprep.registry.Registry._ng_mapping` und `_non_ng_mapping` zeigen weiterhin auf die jeweiligen `processor.<Name>` — keine Änderung.
  3. `tests/unit/processor/<name>/test_<name>.py` läuft **ohne Änderung** grün — Test-Suite ist die Wahrheit.
  4. **Negativkatalog-Check** (CI-Skript `scripts/check_phase4_adapter_thinness.py`):
     ```bash
     # migrierte Prozessoren (in PHASE4_DONE.txt gelistet) dürfen kein _apply_rules / process / Helper importieren
     for name in $(cat scripts/PHASE4_DONE.txt); do
       f="logprep/ng/processor/$name/processor.py"
       grep -E 'pop_dotted_field_value|add_fields_to|get_dotted_field_value|pyparsing|msgspec|^import re|^import base64|_rule_tree\.get_matching_rules|def _apply_rules|async def process' "$f" \
         && { echo "VIOLATION in $f"; exit 1; }
     done
     ```
     Exit-Code ≠ 0 bricht CI. (Un-migrierte Prozessoren behalten `_apply_rules`; sie werden vom Check nicht erfasst.)
  5. `benchmarks/results/phase4_<name>_ng_*.txt` wird mit `benchmarks/run_phase_benchmark.py` aufgenommen; Vergleich gegen `phase3_5_ng_*.txt` darf keine Regression > 5 % zeigen.
  6. `CHANGELOG.md`: Eintrag in „## Upcoming Changes / ### Improvements" pro Processor („`<name>`: migrate rule apply to Rust `RuleSpec` via `ProcessorCore` (Phase 3.5), replace `<external_lib>` with `<rust_crate>`; Python `Rule` retained for API").

### Verifizierung pro Subphase

```bash
# Build + Rust-Unit-Tests
cargo build --release
cargo test -p logprep-core

# Targeted Python-Tests pro Processor (müssen ohne Änderung grün sein)
uv run pytest tests/unit/processor/dropper/ -vvv
uv run pytest tests/unit/processor/deleter/ -vvv
uv run pytest tests/unit/processor/field_manager/ -vvv
uv run pytest tests/unit/processor/concatenator/ -vvv
uv run pytest tests/unit/processor/string_splitter/ -vvv
uv run pytest tests/unit/processor/calculator/ -vvv
uv run pytest tests/unit/processor/dissector/ -vvv
uv run pytest tests/unit/processor/replacer/ -vvv
uv run pytest tests/unit/processor/decoder/ -vvv

# Negativkatalog: Python-Adapter enthält keine Event-Logik
uv run python scripts/check_phase4_adapter_thinness.py

# Coverage
uv run pytest tests/unit/processor/ --cov=logprep --cov-report=xml -vvv

# Qualität
uv run black --check --diff --config ./pyproject.toml .
uv run pylint $(git diff --name-only --diff-filter=ACMR main...HEAD -- '*.py' '*.rs')
uv run mypy $(git diff --name-only --diff-filter=ACMR main...HEAD -- '*.py')

# Extern-Lib-Check: keine neuen Python-Deps, keine msgspec/pyparsing/re/base64 in ng/
uv run python scripts/check_no_external_libs_in_ng.py

# Benchmark
uv run python ./benchmarks/run_phase_benchmark.py \
    --phase phase4_<name> --processors <name> \
    --output benchmarks/results/phase4_<name>_ng_$(date +%Y%m%d_%H%M%S).txt
```

### Akzeptanzkriterien (Phase 4 abgeschlossen)

- [ ] Alle 9 priorisierten Prozessoren (dropper, deleter, field_manager, concatenator, string_splitter, calculator, dissector, replacer, decoder) haben einen `RuleSpec`-Struct in `crates/logprep-core/src/processor/<name>.rs` und sind über den `ProcessorCore`-Slot registriert (kein Python-Callback mehr).
- [ ] Schritt 4i Welle 2: Alle Prozessoren der Gruppen A–C (18) haben einen `RuleSpec` und sind ohne Python-Callback registriert; die Prozessoren der Gruppe D (requester, domain_resolver, generic_resolver, geoip_enricher, amides) sind als bewusste Callback-Ausnahme in `MIGRATION_PLAN.md` und `scripts/PHASE4_DONE.txt` dokumentiert.
- [ ] `field::value` (Pure-Rust-Versionen der Phase-1-Helper) ist in `crates/logprep-core/src/field/value.rs` vorhanden; `field::py` ist dünner Wrapper.
- [ ] **Kein** migriertes `logprep/ng/processor/<name>/processor.py` importiert `pop_dotted_field_value`, `add_fields_to`, `get_dotted_field_value`, `pyparsing`, `msgspec`, `re`, `base64` oder ruft `_rule_tree.get_matching_rules` auf, und definiert kein `_apply_rules` / kein `async def process` (verifiziert via `scripts/check_phase4_adapter_thinness.py`).
- [ ] **Keine** `for`-Schleife über `event.data`-Keys, `if`-Verzweigungen auf Event-Feldern, oder Event-mutierende Operationen in migrierten Python-Adaptern (nur `load_rules`-Registrierung).
- [ ] Die Python-`Rule`-Klassen aller 9 Prozessoren (`logprep/processor/<name>/rule.py`) bleiben **unverändert** als `attrs`-`@define` mit `Config` + `Metrics` — externe API (`rule.drop`, `rule.source_fields`, `rule.metrics`, …) intakt.
- [ ] Metriken werden ausschließlich über `outcome.matched_rule_ids` gepflegt (aus der ABC, Phase 3.5); keine Iteration über alle Rules in `process` (Leitprinzip 7 Vertrag).
- [ ] Externe Python-Bibliotheken für die Event-Verarbeitung ersetzt: `pyparsing` → eigener Pratt-Parser, `msgspec` → `serde_json`, Python-`re` → `regex` Crate, `base64` → `base64` Crate. `attrs` bleibt an Python-Rule-Klassen (Leitprinzip 3).
- [ ] Phase-1-Helper werden in jeder `RuleSpec::apply` über `crate::field::value::*` aufgerufen (kein Reimplementieren).
- [ ] Bestehende Unit-Tests aller 9 Prozessoren laufen ohne Modifikation grün.
- [ ] Rust-Unit-Tests pro Processor: ≥ 20 (dropper, deleter), ≥ 40 (field_manager, calculator, dissector, replacer, decoder), ≥ 50 (concatenator, string_splitter). Plus ≥ 30 für `field::value`.
- [ ] Performance: kein Prozessor zeigt > 5 % Regression ggü. Phase-3.5-Baseline; mindestens die field_manager-/dropper-Klasse zeigen ≥ 10 % Speedup (Callback-Roundtrip entfällt).
- [ ] `pre-commit run --all-files` grün, `mypy`/`pylint`/`black` clean, `uv lock --check` ok, neue CI-Jobs grün.
- [ ] `CHANGELOG.md` enthält Eintrag pro migriertem Prozessor inkl. Crate-Mapping.

### Risiken & Gegenmaßnahmen

| Risiko | Gegenmaßnahme |
|---|---|
| `field::value` muss bytegenau zum Python-Verhalten sein (Merge-Semantik, Drop-Empty-Rekursion, Dotted-Field-Slicing) | Behavior-Preservation-Tests in 4a: pro Funktion ≥ 5 Tests mit Vorher-/Nachher-Vergleich gegen `helper.py` (vor Refactor). `scripts/compare_helpers.py` läuft beide Versionen auf 1000+ zufälligen Events |
| `pyparsing` → eigener Pratt-Parser kann semantische Unterschiede haben (Whitespace, Operator-Precedence, unäres Minus) | Calculator-Tests 1:1 nach Rust spiegeln, jeden `pyparsing.ParseException`-Test in einen `Expr::Err`-Test umwandeln. Spezielle Tests für rechtsassoziatives `^`, chained `==`, `from_hex` mit Hex-Prefix |
| Python-`re` und Rust-`regex` haben subtile Unterschiede (z. B. Unicode-Word-Boundaries, POSIX-vs-ECMAScript) | In 4a Inventur aller in Python-`re`-Verwendungen genutzten Patterns; Rust-`regex` standardmäßig ECMAScript-Flag, das in 99 % der Fälle passt. Explizite Tests für `\b` und Unicode-Klassen |
| `msgspec` JSON-Decoder ist strikter als `serde_json` (z. B. `NaN`/`Infinity` als ungültig) | In 4f: Tests mit denselben Edge-Cases wie die bestehenden Decoder-Tests. Bei Verhaltensunterschied Mapping auf `DecoderError` |
| `attrs`-Validierung ist subtiler als `serde::Deserialize` (z. B. `deep_iterable` über Dicts, bedingte Defaults) | Pro Processor eine vollständige Liste der Edge-Cases (z. B. `drop: []`, `drop_full: None`, `source_fields: None`) als Rust-Unit-Tests, abgeleitet aus den bestehenden Python-Tests |
| `timeout`-Decorator (Python) via Thread+Signal vs. `std::sync::mpsc` (Rust) | 4g: Tests mit `timeout=0.001` (sehr kurz) verifizieren, dass Timeout-Fehler als `ProcessingWarning` ankommen, nicht als Panic. `std::thread::spawn` + `recv_timeout` ist die Standard-Idiom in Rust |
| Round-Trip `dict → serde_json::Value → dict` pro Event | Phase 1 hat den Round-Trip bereits optimiert; Phase 4 nutzt `serde_json::Value` direkt als Event-Container. PyO3 nutzt `Bound<PyDict>::from_owned`/`into_owned` statt `pythonize`-Deepcopy. Benchmark in 4a misst Overhead |
| Metriken: Rust weiß beim `apply` nichts vom Python-`Rule` und seinen `Rule.Metrics` | Rust liefert pro `process()` die `matched_rule_ids` (Phase 3.5 Outcome-Vertrag, Leitprinzip 7). Die ABC (Python) inkrementiert `Rule.metrics.number_of_processed_events` **nur** über die getroffenen IDs — nicht über alle Rules ( korrekt zu `processor.py:146`). Keine Metrik-Logik in `RuleSpec::apply`. Der `matched_rule_ids`-Vertrag ist der spätere Eintrittspunkt für eine Rust-Metrics-Registry (Anforderung 3). Vor dessen Umzug ist `Rule.Metrics` Source of Truth. Edge-Case-Test: Processor mit 0 treffenden Rules → kein Metrik-Inkrement |

---

## Phase 5: Connector-Wrapper (ng/)

**Ziel**: Connector-Interfaces für ng/ in Rust implementieren. I/O-Bibliotheken (confluent-kafka, opensearch-py, boto3) werden durch Rust-Crates ersetzt, wenn verfügbar. Andernfalls wird die Python-Bibliothek via PyO3 gewrapped.

**Begründung**: ng/-Basis (Leitprinzip 8). Rust-Batching, Retry, Circuit-Breaking, Metriken, Fehlerbehandlung. Python-Bibliotheken nur als I/O-Backend wenn kein Rust-Crate verfügbar.

### Rust-Struktur

```
crates/logprep-core/src/connector/
├── mod.rs
├── base.rs           # Connector-Trait + Metriken
├── batch.rs          # Batch-Logik
├── retry.rs          # Retry + Circuit-Breaker
├── jsonl.rs          # jsonl Input/Output (direkt in Rust, kein Python)
├── http.rs           # reqwest statt aiohttp
├── opensearch.rs     # opensearch-Rust-Client statt opensearch-py
└── kafka.rs          # rdkafka statt confluent-kafka
```

### Rust-Crates-Entscheidungen

| Python-Bibliothek | Rust-Crate | Status |
|---|---|---|
| `aiohttp` | `reqwest` | ✅ Verfügbar |
| `opensearch-py` | `opensearch` | ✅ Verfügbar |
| `confluent-kafka` | `rdkafka` | ✅ Verfügbar |
| `boto3` | `aws-sdk-s3` | ✅ Verfügbar |

### Priorisierung

1. `jsonl_input` / `jsonl_output` (Datei-I/O, direkt in Rust)
2. `http_input` / `http_output` (reqwest statt aiohttp)
3. `opensearch_output` (opensearch-Rust-Client)
4. `confluentkafka_*` (rdkafka)

### Python-Brücke

```python
# logprep/ng/connector/
from logprep._rust.connector import (
    JsonlInput, JsonlOutput,
    HttpInput, HttpOutput,
    OpenSearchOutput,
    ConfluentKafkaInput, ConfluentKafkaOutput,
)
```

### Verifizierung

```bash
cargo test -p logprep-core
uv run pytest tests/unit/connector/ -vvv
uv run pytest tests/acceptance/ -vvv
```

### Performance-Test

```bash
uv run python ./benchmarks/benchmark_connectors.py --baseline benchmarks/phase4.json \
  --output benchmarks/phase5.json
```

---

## Phase 5.5 (Nach Phase 5, parallel zu 4c–4i): Rückbau des Event-Serialisierungs-Overheads

**Ziel**: Die in 4a bewusst in Kauf genommenen `dict → serde_json::Value → dict`-Roundtrips werden schrittweise vollständig abgebaut, bis im ng-Event-Pfad kein Event mehr für Helfer-Aufrufe konvertiert wird.

**Hintergrund**: Der Phase-4-Benchmark (`benchmarks/results/phase4_ng_20260918_150753.txt`, weighted 2.648 docs/s) zeigt **−17,2 % ggü. Phase 3.5**. Ursache: `field::py` delegiert plangetreu auf `field::value`; jeder Helfer-Aufruf aus den Python-`_apply_rules`-Callbacks der noch un-migrierten Prozessoren (und `delete_source_fields` im `ProcessorCore`, je gepopptem Source-Feld ein Full-Rebuild des Live-Dicts) zahlt O(ganzes Event) pro Aufruf. Dieser Overhead ist temporär und sinkt mit jedem migrierten `RuleSpec`, da der Callback-Pfad entfällt.

**Aufgaben**:

- **5.5a — `delete_source_fields` im Core optimieren**: alle Source-Felder einmalig auf dem bereits konvertierten `Value` poppen und erst danach einmal zurück ins PyDict synchronisieren (statt Voll-Roundtrip pro Feld).
- **5.5b — Callback-Pfad trockengelegt**: mit Abschluss von 4c–4i rufen nur noch die Prozessoren der Gruppe D (requester, domain_resolver, generic_resolver, geoip_enricher, amides) `field::py`-Helfer pro Event auf; Inventur via Negativkatalog-Check (`scripts/check_phase4_adapter_thinness.py`), Liste der verbleibenden Nutzer von `logprep._rust`-Feldhelfern im ng-Pfad.
- **5.5c — `field::py` auf Legacy-Pfad zurückschneiden**: sobald `ng/` keinen Roundtrip mehr nutzt, verbleiben die PyO3-Helfer nur für den nicht-ng-Legacy-Pfad; dort kann der Live-Dict-Zugriff (ohne Konvertierung) als zweite Implementierung erhalten bleiben, bis der Legacy-Pfad eingestellt wird.
- **5.5d — Verifizierung**: Phase-Benchmark gegen die 3.5-Baseline (`benchmarks/results/phase3_5_ng_20260731_150656.txt`, 3.200 docs/s weighted); Akzeptanz: ≥ 3.5-Niveau, soweit die Benchmark-Pipeline keine Prozessoren der Gruppe D enthält (deren Roundtrip-Anteil ist als verbleibende Differenz zu dokumentieren), Ergebnis in `benchmarks/BENCHMARK_HISTORY.md` eintragen.

---

## Phase 6: Pipeline-Orchestrierung (ng/)

**Ziel**: `Pipeline` und `PipelineManager` für ng/ komplett in Rust mit tokio implementieren.

**Begründung**: ng/-Basis (Leitprinzip 8). Tokio als Async-Runtime statt uvloop. Python-Wrapper ist minimal.

### Rust-Struktur

```
crates/logprep-core/src/pipeline/
├── mod.rs
├── pipeline.rs        # Pipeline (Event-Loop mit tokio)
└── manager.rs         # PipelineManager (Multiprocessing)
```

### Python-Brücke

```python
# logprep/ng/manager.py
from logprep._rust.pipeline import PipelineManager

class PipelineManager:
    def __init__(self, configuration):
        self._inner = PipelineManager._from_config(configuration)
```

### Verifizierung

```bash
cargo test -p logprep-core
uv run pytest tests/unit/framework/ -vvv
```

### Performance-Test

```bash
uv run python ./benchmarks/benchmark_pipeline.py --baseline benchmarks/phase5.json \
  --output benchmarks/phase6.json
```

---

## Phase 7: Runner + CLI (ng/)

**Ziel**: `Runner` und CLI für ng/ in Rust mit clap implementieren.

**Begründung**: ng/-Basis (Leitprinzip 8). Letzter Schritt — alle vorherigen Phasen sind stabil.

### Rust-Struktur

```
crates/logprep-core/src/
├── runner/
│   ├── mod.rs
│   ├── runner.rs      # Runner (Config-Refresh, Stop-Event)
│   └── signal.rs      # Signal-Handler
├── cli/
│   └── mod.rs         # clap-basierte CLI
```

### Python-Brücke

```python
# logprep/run_ng.py (minimal)
from logprep._rust.runner import Runner
from logprep._rust.cli import cli

if __name__ == "__main__":
    cli()
```

### Verifizierung

```bash
cargo test -p logprep-core
uv run pytest tests/unit/test_run_ng.py -vvv
uv run logprep run config.yaml
```

### Performance-Test

```bash
uv run python ./benchmarks/benchmark_full_pipeline.py --baseline benchmarks/phase6.json \
  --output benchmarks/phase7.json
```

---

## Abhängigkeitskette

```
Phase 1 (Setup + Dotted-Field)
  └─> Phase 2 (Filter)
       └─> Phase 3 (RuleTree)
            └─> Phase 3.5 (Processor-Orchestrierung / Base-ABC)
                 └─> Phase 4 (Processor-RuleSpecs)
                      └─> Phase 5 (Connectors, ng/)
                           └─> Phase 5.5 (Rückbau Serialisierungs-Overhead, parallel zu 4c–4i)
                                └─> Phase 6 (Pipeline, ng/)
                                     └─> Phase 7 (Runner + CLI, ng/)
```

Phase 3.5 ist die Voraussetzung für Anforderung 1 (intern nur noch Rust): sie migriert die gemeinsame Orchestrierung *vor* der per-Prozessor-Migration. Phase 4 belegt dann pro Processor den `rule_specs`-Slot des Cores. Phase 5.5 baut den in 4a bewusst in Kauf genommenen Event-Roundtrip-Overhead wieder ab, sobald die migrierten `RuleSpec`s den Python-Callback-Pfad ersetzen.

Jede Phase baut auf der vorherigen auf. Nach jeder Phase:
1. Alle Tests grün
2. `pre-commit run --all-files` bestanden
3. Applikation weiterhin ausführbar
4. Performance-Test durchgeführt (`./benchmarks`)
5. CHANGELOG.md aktualisiert
6. Merge in `main`

---

## Risiken & Gegenmaßnahmen

| Risiko | Gegenmaßnahme |
|---|---|
| PyO3-Overhead für kleine Funktionen | Nur Hot-Paths in Rust; I/O bleibt in Python |
| Python-Libs nicht in Rust nutzbar | Rust-Crates bevorzugen; PyO3-Wrapper nur als Fallback |
| Async-Kompatibilität (tokio ↔ uvloop) | ng/ nutzt direkt tokio; pyo3-asyncio für Bridge |
| Test-Abdeckung | Jede Phase muss alle bestehenden Tests bestehen |
| Build-Komplexität | maturin für nahtlose Integration; CI erweitern |
| Performance-Regression | Benchmark nach jeder Phase; Baseline-Vergleich |

---

## CI/CD-Anpassungen

```yaml
# .github/workflows/rust.yml (neu)
- name: Build Rust
  run: maturin build --release

- name: Test Rust
  run: cargo test --workspace

- name: Test Python (with Rust)
  run: uv run pytest ./tests --cov=logprep --cov-report=xml -vvv

- name: Performance Benchmark
  run: uv run python ./benchmarks/run_all.py
  if: github.event_name == 'pull_request'
```
