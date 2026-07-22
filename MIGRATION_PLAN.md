# Logprep Rust-Migration (PyO3)

**Ziel**: Schrittweise Migration der ng/ (async, uvloop) Codebasis nach Rust über PyO3.
Die Applikation muss zu jedem Zeitpunkt weiter ausführbar bleiben.

## Leitprinzipien

- Nicht-alles-auf-einmal: Jede Phase liefert nutzbare Rust-Funktionen, die von Python aus aufrufbar sind
- Python-Brücke: Rust-Module werden als `logprep._rust` (PyO3) importiert
- Kein Big Bang: Bestehende Python-Implementierungen bleiben erhalten, bis die Rust-Äquivalente getestet sind
- ng/ als Basis: Alle Rust-Komponenten werden für die ng/ (async) Codebasis entwickelt
- 100% Testabdeckung
- Rust Tests für Rust Funktionen existieren neben python Tests für die Integration

---

## Phase 1: PyO3-Setup + Erste Funktion

**Ziel**: Rust-Build-Pipeline etablieren, dotted-field Helper in Rust implementieren.

### Setup

```
logprep/
├── Cargo.toml                 # Workspace-Rooting
├── crates/
│   └── logprep-core/
│       ├── Cargo.toml         # pyo3, tokio, serde, serde_json
│       └── src/
│           ├── lib.rs
│           └── field.rs       # get_dotted_field_value, has_dotted_field, pop_dotted_field_value, add_fields_to
├── pyproject.toml             # maturin build-backend
└── logprep/_rust/
    ├── __init__.py            # Re-exports
    └── py.typed               # Type-Stub
```

### Erste Rust-Funktionen (`logprep/util/helper.py`)

- `get_dotted_field_value(event, dotted_field)` — überall verwendet, rein berechnungsintensiv
- `has_dotted_field(event, dotted_field)` — Existenzprüfung
- `pop_dotted_field_value(event, dotted_field, drop_full)` — Feld-Entfernung
- `add_fields_to(event, fields, merge, overwrite)` — Feld-Hinzufügen

### Python-Integration

```python
# logprep/util/helper.py
from logprep._rust import get_dotted_field_value  # Rust-Implementierung
```

### Verifizierung

```bash
cargo build --release && cargo test -p logprep-core
uv run pytest tests/unit/util/test_helper.py -vvv
pre-commit run --all-files
```

---

## Phase 2: Filter-Engine (FilterExpression AST)

**Ziel**: Filter-Expression AST + Lucene-Parser in Rust.

**Begründung**: Reine Logik, Hot Path für jedes Rule-Matching, keine externen I/O-Abhängigkeiten.

### Rust-Struktur

```
crates/logprep-core/src/filter/
├── mod.rs
├── expression.rs    # FilterExpression Enum + Match-Logik
├── lucene.rs         # Lucene-Query-Parser
└── range.rs          # Range-Typen
```

**Enthält**: `StringFilterExpression`, `WildcardFilterExpression`, `SigmaFilterExpression`,
`IntegerFilterExpression`, `FloatFilterExpression`, `RangeExpression`, `RegExFilterExpression`,
`Exists`, `Null`, `And`, `Or`, `Not`

### Verifizierung

```bash
cargo test -p logprep-core
uv run pytest tests/unit/filter/ -vvv
```

---

## Phase 3: Rule Tree + Rule Matching

**Ziel**: `RuleTree` und `Rule`-Matching in Rust.

**Begründung**: Zentraler Matching-Mechanismus, wird von jedem `Processor.process()` aufgerufen.

### Rust-Struktur

```
crates/logprep-core/src/rule/
├── mod.rs
├── tree.rs           # RuleTree (Baum-Struktur)
├── segment.rs        # Rule-Segmentierung
└── matcher.rs        # Matching-Logik
```

### Verifizierung

```bash
cargo test -p logprep-core
uv run pytest tests/unit/framework/rule_tree/ -vvv
```

---

## Phase 4: Processor-Core (Einfache Processor)

**Ziel**: Einfache, rechenintensive Processor in Rust implementieren.

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
        // Rust-Logik
    }
}
```

### Verifizierung

```bash
cargo test -p logprep-core
uv run pytest tests/unit/processor/ -vvv
```

---

## Phase 5: Connector-Wrapper

**Ziel**: Connector-Interfaces in Rust, I/O via Python-Bibliotheken (confluent-kafka, opensearch-py, boto3).

**Rust-seitig**: Batching, Retry, Circuit-Breaking, Metriken, Fehlerbehandlung.

### Rust-Struktur

```
crates/logprep-core/src/connector/
├── mod.rs
├── base.rs           # Connector-Trait + Metriken
├── batch.rs          # Batch-Logik
└── retry.rs          # Retry + Circuit-Breaker
```

### Priorisierung

1. `jsonl_input` / `jsonl_output` (Datei-I/O, direkt in Rust)
2. `http_input` / `http_output` (aiohttp-Wrapper)
3. `opensearch_output` (opensearch-py-Wrapper)
4. `confluentkafka_*` (confluent-kafka-Wrapper)

### Verifizierung

```bash
cargo test -p logprep-core
uv run pytest tests/unit/connector/ -vvv
uv run pytest tests/acceptance/ -vvv
```

---

## Phase 6: Pipeline-Orchestrierung

**Ziel**: `Pipeline` und `PipelineManager` in Rust mit tokio.

### Rust-Struktur

```
crates/logprep-core/src/pipeline/
├── mod.rs
├── pipeline.rs        # Pipeline (Event-Loop)
└── manager.rs         # PipelineManager (Multiprocessing)
```

### Integration

```python
# logprep/ng/manager.py
from logprep._rust.pipeline import PipelineManager

class PipelineManager:
    def __init__(self, configuration):
        self._inner = RustPipelineManager(configuration)
```

### Verifizierung

```bash
cargo test -p logprep-core
uv run pytest tests/unit/framework/ -vvv
```

---

## Phase 7: Runner + CLI

**Ziel**: `Runner` und CLI (clap statt click) in Rust.

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

### Integration

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

---

## Abhängigkeitskette

```
Phase 1 (Setup + Dotted-Field)
  └─> Phase 2 (Filter)
       └─> Phase 3 (RuleTree)
            └─> Phase 4 (Processor)
                 └─> Phase 5 (Connectors)
                      └─> Phase 6 (Pipeline)
                           └─> Phase 7 (Runner + CLI)
```

Jede Phase baut auf der vorherigen auf. Nach jeder Phase:
1. Alle Tests grün
2. `pre-commit run --all-files` bestanden
3. Applikation weiterhin ausführbar
4. CHANGELOG.md aktualisiert
5. Merge in `main`

---

## Risiken & Gegenmaßnahmen

| Risiko | Gegenmaßnahme |
|---|---|
| PyO3-Overhead für kleine Funktionen | Nur Hot-Paths in Rust; I/O bleibt in Python |
| Python-Libs (confluent-kafka) nicht in Rust nutzbar | Wrapper-Schicht via PyO3 |
| Async-Kompatibilität (tokio ↔ uvloop) | pyo3-asyncio für Bridge; tokio-Laufzeit in eigenem Thread |
| Test-Abdeckung | Jede Phase muss alle bestehenden Tests bestehen |
| Build-Komplexität | maturin für nahtlose Integration; CI erweitern |

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
```
