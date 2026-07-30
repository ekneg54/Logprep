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

## Phase 4: Processor-Core (Einfache Processor)

**Ziel**: Einfache, rechenintensive Processor komplett in Rust implementieren, mit Python-Brücke für `logprep/ng/`.

**Begründung**: Alle Prozessoren nutzen die Rust-Helfer (Phase 1) und den Rust-Filter (Phase 2). Die ng/-Basis wird direkt in Rust entwickelt.

### Priorisierte Processor (niedrige externe Abhängigkeiten)

| Processor | Begründung |
|---|---|
| `dropper` | Simpleste Logik, Referenz |
| `deleter` | Ähnlich wie dropper |
| `field_manager` | Zentrale Feldmanipulation |
| `concatenator` | String-Konkatenation |
| `string_splitter` | String-Operationen |
| `calculator` | Numerische Berechnungen |
| `dissector` | Pattern-basierte Aufteilung |
| `replacer` | String-Ersetzung |
| `decoder` | Base64/Hex-Decoding |

### Architektur

Jeder Processor als PyO3-Klasse mit `process(event)` Methode:

```rust
#[pyclass]
pub struct Dropper {
    name: String,
    rules: Vec<Rule>,
}

#[pymethods]
impl Dropper {
    #[new]
    fn new(name: String, config: PyObject) -> PyResult<Self> { ... }

    fn process(&self, py: Python, event: PyObject) -> PyResult<PyObject> {
        // Rust-Logik komplett in Rust
    }
}
```

### Python-Brücke

```python
# logprep/ng/processor/dropper/processor.py
from logprep._rust.processor import Dropper
```

### Verifizierung

```bash
cargo test -p logprep-core
uv run pytest tests/unit/processor/ -vvv
```

### Performance-Test

```bash
uv run python ./benchmarks/benchmark_processors.py --baseline benchmarks/phase3.json \
  --output benchmarks/phase4.json
```

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
            └─> Phase 4 (Processor)
                 └─> Phase 5 (Connectors, ng/)
                      └─> Phase 6 (Pipeline, ng/)
                           └─> Phase 7 (Runner + CLI, ng/)
```

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
