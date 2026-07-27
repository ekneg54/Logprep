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

**Ziel**: Rust-Build-Pipeline etablieren, **alle** Dotted-Field-Funktionen aus `logprep/util/helper.py` nach Rust migrieren. Nach Phase 1 enthält `helper.py` nur noch thin Python-Wrapper und nicht-dotted-field Hilfsfunktionen.

**Begründung**: Die 24 Dotted-Field-Funktionen bilden das Fundament aller Prozessoren. Sie sind rein rechenintensiv, haben keine I/O-Abhängigkeiten und werden von fast jeder Datei genutzt. Eine vollständige Migration in Phase 1 vermeidet gemischte Python/Rust-Logik über mehrere Phasen.

**Wichtig**: Nach dem Wechsel zu maturin existiert **kein Python-Fallback**. Die Rust-Bibliothek ist zwingende Build-Dependency. `try/except ImportError`-Blöcke sind nicht erlaubt (Leitprinzip 7).

### Funktionsübersicht: Alle 24 Dotted-Field Helper

| # | Funktion | Kategorie | Abhängigkeiten |
|---|---|---|---|
| 1 | `get_dotted_field_list` | Parsing | — |
| 2 | `field_list_to_dotted_field` | Parsing (reverse) | — |
| 3 | `join_dotted_fields` | Parsing (reverse) | — |
| 4 | `_get_slice_arg` | Low-Level | — |
| 5 | `_get_item` | Low-Level | `_get_slice_arg` |
| 6 | `get_dotted_field_value` | Read | `get_dotted_field_list`, `_get_item` |
| 7 | `get_dotted_field_value_with_missing` | Read | `get_dotted_field_list`, `_get_item` |
| 8 | `get_field_value` | Read | `_get_item` |
| 9 | `get_field_value_no_slice` | Read (optimiert) | — |
| 10 | `get_dotted_field_values` | Read (Batch) | `get_dotted_field_value_with_missing` |
| 11 | `has_dotted_field` | Existenz | `get_dotted_field_value`, `get_dotted_field_value_with_missing` |
| 12 | `_pop_field_value` | Pop | `get_dotted_field_list`, `get_field_value` |
| 13 | `_pop_field_value_and_drop_empty` | Pop | `get_dotted_field_list` |
| 14 | `pop_dotted_field_value` | Pop | `_pop_field_value`, `_pop_field_value_and_drop_empty` |
| 15 | `_add_and_overwrite_key` | Write | — |
| 16 | `_add_and_not_overwrite_key` | Write | — |
| 17 | `_add_field_to` | Write | `get_dotted_field_list`, `_add_and_overwrite_key`, `_add_and_not_overwrite_key` |
| 18 | `_add_field_to_silent_fail` | Write | `_add_field_to` |
| 19 | `add_fields_to` | Write (Batch) | `_add_field_to`, `_add_field_to_silent_fail` |
| 20 | `append_as_list` | Thin Wrapper | `add_fields_to` (partial) |
| 21 | `add_and_overwrite` | Thin Wrapper | `add_fields_to` |
| 22 | `copy_fields_to_event` | Thin Wrapper | `get_dotted_field_values`, `add_fields_to` |
| 23 | `append` | Thin Wrapper | `get_dotted_field_value`, `add_and_overwrite`, `append_as_list` |
| 24 | `get_source_fields_dict` | Thin Wrapper | `get_dotted_field_value` |

### Python-Modell nach Phase 1

`logprep/util/helper.py` enthält danach:
- **Thin Wrapper** (20-24): `append_as_list`, `add_and_overwrite`, `copy_fields_to_event`, `append`, `get_source_fields_dict` — jeweils 1-3 Zeilen Python-Logik
- **Nicht-dotted-field Helfer**: `Missing`, `SKIP`, `DottedTemplate`, `recursive_compare`, `remove_file_if_exists`, `camel_to_snake`, `snake_to_camel`, `get_versions_string`, `deduplicate_with_order`, `resolve_template`, `create_template_resolver`, `reduce_field_value`, `transform_field_value`
- **Keine Python-Implementierung** mehr für Funktionen 1-19

### Abhängigkeiten

```
1a (Rust scaffolding)
 └─> 1b (maturin + nix)
      └─> 1c (Parsing: get_dotted_field_list, field_list_to_dotted_field, join_dotted_fields)
           └─> 1d (Read: _get_item, get_dotted_field_value, get_dotted_field_value_with_missing,
           │        get_field_value, get_field_value_no_slice, get_dotted_field_values)
           │    └─> 1e (Existenz: has_dotted_field)
           └─> 1f (Pop: _pop_field_value, _pop_field_value_and_drop_empty, pop_dotted_field_value)
           └─> 1g (Write: _add_and_*_key, _add_field_to, _add_field_to_silent_fail, add_fields_to)
                └─> 1h (Thin Wrapper: append_as_list, add_and_overwrite, copy_fields_to_event,
                         append, get_source_fields_dict)
```

Jeder Schritt liefert einen lauffähigen Codestand mit allen Tests grün.

---

### Schritt 1a: Rust-Scaffolding

**Ziel**: Cargo-Workspace anlegen, ohne Python zu beeinflussen.

**Dateien** (rein additiv, kein Bestandscode betroffen):

```
logprep/
├── Cargo.toml                        # workspace: ["crates/*"]
└── crates/
    └── logprep-core/
        ├── Cargo.toml                # [lib] name = "logprep_core"
        └── src/
            ├── lib.rs                # PyO3-Moduldefinition
            └── field.rs              # Dotted-Field Funktionen (initial leer)
```

**Cargo.toml (Workspace-Root):**
```toml
[workspace]
members = ["crates/*"]
resolver = "2"

[workspace.dependencies]
pyo3 = { version = "0.25", features = ["extension-module"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

**crates/logprep-core/Cargo.toml:**
```toml
[package]
name = "logprep-core"
version = "0.1.0"
edition = "2024"

[lib]
name = "logprep_core"
crate-type = ["cdylib"]

[dependencies]
pyo3.workspace = true
serde.workspace = true
serde_json.workspace = true

[profile.release]
lto = true
codegen-units = 1
```

**Verifizierung:**
```bash
cargo build --release -p logprep-core
cargo test -p logprep-core
uv run pytest ./tests -vvv   # Python unberührt
```

**Nix**: Kein Einfluss — die Rust-Dateien werden von nix nicht gebaut (kein pyproject.toml-Input).

---

### Schritt 1b: maturin als Build-Backend + Nix-Integration

**Ziel**: Python-Build-Backend von `uv_build` auf `maturin` umstellen, Nix für Rust-Toolchain erweitern.

#### pyproject.toml Änderungen

```diff
 [build-system]
-requires = ["uv_build>=0.11.6,<0.12"]
-build-backend = "uv_build"
+requires = ["maturin>=1.8,<2"]
+build-backend = "maturin"
+
+[tool.maturin]
+features = ["pyo3/extension-module"]
+python-source = "."
+module-name = "logprep._rust"

-[tool.uv.build-backend]
-module-name = "logprep"
-module-root = ""
```

`uv lock` neu ausführen:
```bash
uv lock
```

Die `uv.lock`-Datei wird jetzt maturin als Build-Dependency enthalten.

#### flake.nix Anpassungen

Das Rust-Toolchain-Input muss hinzugefügt werden. `pyproject-build-systems` liefert maturin bereits als Python-Paket, aber `cargo`/`rustc` müssen im Build-Environment verfügbar sein.

```nix
# Neue Flake-Inputs
inputs = {
  # ... bestehende Inputs ...
  rust-overlay = {
    url = "github:oxalica/rust-overlay";
    inputs.nixpkgs.follows = "nixpkgs";
  };
};

# In let-Block:
let
  # ... bestehende Config ...
  pkgs = import nixpkgs {
    system = "x86_64-linux";
    overlays = [ rust-overlay.overlays.default ];
  };
  rustToolchain = pkgs.rust-bin.stable.latest.default;
in
```

Die devShells und package-Builds müssen `rustToolchain` als Build-Input bekommen:

```nix
# In mkShellFor:
pkgs.mkShell {
  packages = [
    # ... bestehende Pakete ...
    rustToolchain
    pkgs.maturin
  ];
};

# In pythonSet aufruf — maturin braucht cargo/rustc:
maturinBuildSystem = pythonSet: pythonSet.overrideScope (final: prev: {
  maturin = prev.maturin.override {
    buildInputs = [ rustToolchain ];
  };
});
```

**Verifizierung:**
```bash
uv sync --frozen --extra dev
cargo test -p logprep-core
uv run pytest ./tests -vvv
nix build .#packages.x86_64-linux.python312
```

**Performance-Test:**
```bash
# Baseline mit Python-Implementierung sichern (vor Rust-Aktivierung)
uv run python ./benchmarks/benchmark_helpers.py --output benchmarks/baseline_phase1b.json
```

**Container-Build mit Nix:**
```bash
nix build .#packages.x86_64-linux.docker.python312 -o image
docker load < image
```

---

### Schritt 1c: Parsing + Joining in Rust

**Ziel**: Die drei Parser-Funktionen migrieren — das Fundament für alle folgenden Funktionen.

**Rust-Implementierung** (`crates/logprep-core/src/field.rs`):

```rust
use pyo3::prelude::*;

/// Splitted einen Dotted-Field-String in seine Komponenten.
/// Unterstützt Escaping: "dotted\.field" → ["dotted.field"]
#[pyfunction]
fn get_dotted_field_list(dotted_field: &str) -> Vec<String> {
    if !dotted_field.contains('\\') {
        return dotted_field.split('.').map(String::from).collect();
    }

    let mut result = Vec::new();
    let mut char_buffer = String::new();
    let mut chars = dotted_field.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '.' => result.push(std::mem::take(&mut char_buffer)),
            '\\' => match chars.next() {
                Some(next) => char_buffer.push(next),
                None => char_buffer.push('\\'),
            },
            _ => char_buffer.push(c),
        }
    }
    result.push(char_buffer);
    result
}

/// Kombiniert eine Feld-Liste zu einem Dotted-Field.
/// Punkte in Feldnamen werden escappt: ["x.y", "z"] → "x\\.y.z"
#[pyfunction]
fn field_list_to_dotted_field(field_list: Vec<String>) -> String {
    field_list
        .iter()
        .map(|field| field.replace(".", "\\."))
        .collect::<Vec<_>>()
        .join(".")
}

/// Kombiniert Dotted-Fields ohne Escaping: ["x.y", "z"] → "x.y.z"
#[pyfunction]
fn join_dotted_fields(dotted_fields: Vec<String>) -> String {
    dotted_fields.join(".")
}
```

**PyO3-Modulstruktur** (`crates/logprep-core/src/lib.rs`):

```rust
use pyo3::prelude::*;

mod field;

#[pymodule]
fn _rust(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(field::get_dotted_field_list, m)?)?;
    m.add_function(wrap_pyfunction!(field::field_list_to_dotted_field, m)?)?;
    m.add_function(wrap_pyfunction!(field::join_dotted_fields, m)?)?;
    Ok(())
}
```

**Migration in `logprep/util/helper.py`:**

```python
from logprep._rust import (
    get_dotted_field_list,
    field_list_to_dotted_field,
    join_dotted_fields,
)  # noqa: F401
```

**Wichtig**: Der `lru_cache` auf `get_dotted_field_list` wird entfernt — Rust ist schneller ohne Cache.

**Rust-Tests:**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_dotted_field() {
        assert_eq!(get_dotted_field_list("a.b.c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn escaped_dot() {
        assert_eq!(get_dotted_field_list(r"dotted\.field"), vec!["dotted.field"]);
    }

    #[test]
    fn double_escaped() {
        assert_eq!(get_dotted_field_list(r"dotted\\.field"), vec![r"dotted\", "field"]);
    }

    #[test]
    fn no_dots() {
        assert_eq!(get_dotted_field_list("simple"), vec!["simple"]);
    }

    #[test]
    fn trailing_backslash() {
        assert_eq!(get_dotted_field_list(r"field\\"), vec![r"field\"]);
    }

    #[test]
    fn field_list_to_dotted_field_simple() {
        assert_eq!(field_list_to_dotted_field(vec!["a".into(), "b".into(), "c".into()]), "a.b.c");
    }

    #[test]
    fn field_list_to_dotted_field_escape() {
        assert_eq!(field_list_to_dotted_field(vec!["x.y".into(), "z".into()]), "x\\.y.z");
    }

    #[test]
    fn join_dotted_fields_simple() {
        assert_eq!(join_dotted_fields(vec!["x.y".into(), "z".into()]), "x.y.z");
    }
}
```

**Verifizierung:**
```bash
cargo test -p logprep-core
uv run pytest tests/unit/util/test_helper.py -vvv
pre-commit run --all-files
```

**Performance-Test:**
```bash
uv run python ./benchmarks/benchmark_helpers.py --filter get_dotted_field_list,field_list_to_dotted_field,join_dotted_fields \
  --baseline benchmarks/baseline_phase1b.json --output benchmarks/phase1c.json
```

---

### Schritt 1d: Read-Operationen in Rust

**Ziel**: Alle Read-Funktionen migrieren — `_get_item`, `get_dotted_field_value`, `get_dotted_field_value_with_missing`, `get_field_value`, `get_field_value_no_slice`, `get_dotted_field_values`.

**Rust-Implementierung** (`crates/logprep-core/src/field.rs`):

```rust
use pyo3::types::{PyDict, PyList, PySlice};

// --- _get_slice_arg (intern) ---
fn get_slice_arg(slice_item: &str) -> PyResult<Option<isize>> {
    if slice_item.is_empty() {
        return Ok(None);
    }
    slice_item
        .parse::<isize>()
        .map(Some)
        .map_err(|_| pyo3::exceptions::PyValueError::new_err(format!("Invalid slice arg: {}", slice_item)))
}

// --- _get_item (intern, nicht exportiert) ---
fn get_item<'py>(
    py: Python<'py>,
    container: &Bound<'py, pyo3::PyAny>,
    key: &str,
) -> PyResult<Bound<'py, pyo3::PyAny>> {
    // Dict-Zugriff
    if let Ok(dict) = container.downcast::<PyDict>() {
        return dict.get_item(key)?.ok_or_else(|| {
            pyo3::exceptions::PyKeyError::new_err(key.to_string())
        });
    }

    // List-Zugriff: Index oder Slice
    if let Ok(list) = container.downcast::<PyList>() {
        if key.contains(':') {
            let parts: Vec<&str> = key.split(':').collect();
            let start = get_slice_arg(parts.get(0).copied().unwrap_or(""))?;
            let stop = get_slice_arg(parts.get(1).copied().unwrap_or(""))?;
            let step = get_slice_arg(parts.get(2).copied().unwrap_or(""))?.unwrap_or(1);
            let slice = PySlice::new(py, start, stop, Some(step))?;
            return list.get_item(&slice);
        }
        let index: isize = key.parse().map_err(|_| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid index: {}", key))
        })?;
        return list.get_item(index);
    }

    Err(pyo3::exceptions::PyTypeError::new_err(
        "Container is neither dict nor list",
    ))
}

// --- get_dotted_field_value ---
#[pyfunction]
fn get_dotted_field_value(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    dotted_field: &str,
) -> PyResult<PyObject> {
    let parts = get_dotted_field_list(dotted_field);
    let mut current = event.clone();
    for part in &parts {
        current = match get_item(py, &current, part) {
            Ok(val) => val,
            Err(_) => return Ok(py.None()),
        };
    }
    Ok(current.unbind())
}

// --- get_dotted_field_value_with_missing ---
#[pyfunction]
fn get_dotted_field_value_with_missing(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    dotted_field: &str,
) -> PyResult<PyObject> {
    let parts = get_dotted_field_list(dotted_field);
    let mut current = event.clone();
    for part in &parts {
        current = get_item(py, &current, part)?;
    }
    Ok(current.unbind())
}

// --- get_field_value ---
#[pyfunction]
fn get_field_value(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    fields: Vec<String>,
) -> PyResult<PyObject> {
    let mut current = event.clone();
    for field in &fields {
        current = get_item(py, &current, field)?;
    }
    Ok(current.unbind())
}

// --- get_field_value_no_slice ---
#[pyfunction]
fn get_field_value_no_slice(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    fields: Vec<String>,
) -> PyResult<PyObject> {
    let mut current = event.clone();
    for field in &fields {
        if let Ok(dict) = current.downcast::<PyDict>() {
            current = dict.get_item(field)?.ok_or_else(|| {
                pyo3::exceptions::PyKeyError::new_err(field.to_string())
            })?;
        } else {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "Container is not a dict (slicing not supported in no_slice variant)",
            ));
        }
    }
    Ok(current.unbind())
}

// --- get_dotted_field_values (Batch) ---
#[pyfunction]
#[pyo3(signature = (event, dotted_fields, on_missing=None))]
fn get_dotted_field_values(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    dotted_fields: Vec<String>,
    on_missing: Option<PyObject>,
) -> PyResult<PyObject> {
    let result = PyDict::new(py);
    for field_name in &dotted_fields {
        match get_dotted_field_value(py, event, field_name) {
            Ok(value) => {
                if value.is_none(py) {
                    // Prüfe ob das Feld überhaupt existiert
                    let parts = get_dotted_field_list(field_name);
                    let mut current = event.clone();
                    let mut found = true;
                    for part in &parts {
                        match get_item(py, &current, part) {
                            Ok(val) => current = val,
                            Err(_) => { found = false; break; }
                        }
                    }
                    if found {
                        // Feld existiert mit None-Wert
                        result.set_item(field_name, value)?;
                    } else if let Some(ref callback) = on_missing {
                        let fallback = callback.call1(py, (field_name,))?;
                        // SKIP-Check: imported von Python
                        let skip = py.import("logprep.util.helper")?.getattr("SKIP")?;
                        if !fallback.bind(py).eq(skip)? {
                            result.set_item(field_name, fallback)?;
                        }
                    } else {
                        result.set_item(field_name, py.None())?;
                    }
                } else {
                    result.set_item(field_name, value)?;
                }
            }
            Err(_) => {
                if let Some(ref callback) = on_missing {
                    let fallback = callback.call1(py, (field_name,))?;
                    let skip = py.import("logprep.util.helper")?.getattr("SKIP")?;
                    if !fallback.bind(py).eq(skip)? {
                        result.set_item(field_name, fallback)?;
                    }
                } else {
                    result.set_item(field_name, py.None())?;
                }
            }
        }
    }
    Ok(result.into_any().unbind())
}
```

**Python-Migration** (`logprep/util/helper.py`):

```python
from logprep._rust import (
    get_dotted_field_value,
    get_dotted_field_value_with_missing,
    get_field_value,
    get_field_value_no_slice,
    get_dotted_field_values,
)  # noqa: F401
```

`_get_item` und `_get_slice_arg` werden komplett aus Python entfernt — sie existieren nur noch in Rust.

**Verifizierung:**
```bash
cargo test -p logprep-core
uv run pytest tests/unit/util/test_helper.py -vvv
```

**Performance-Test:**
```bash
uv run python ./benchmarks/benchmark_helpers.py \
  --filter get_dotted_field_value,get_dotted_field_value_with_missing,get_field_value,get_dotted_field_values \
  --baseline benchmarks/phase1c.json --output benchmarks/phase1d.json
```

---

### Schritt 1e: `has_dotted_field` in Rust

**Ziel**: Existenz-Check migrieren.

**Rust-Implementierung** (`crates/logprep-core/src/field.rs`):

```rust
/// Prüft ob ein Dotted-Field im Event existiert.
#[pyfunction]
#[pyo3(signature = (event, dotted_field, allow_none=true))]
fn has_dotted_field(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    dotted_field: &str,
    allow_none: bool,
) -> PyResult<bool> {
    let parts = get_dotted_field_list(dotted_field);
    let mut current = event.clone();

    for part in &parts {
        current = match get_item(py, &current, part) {
            Ok(val) => val,
            Err(_) => return Ok(false),
        };
    }

    if allow_none {
        Ok(!current.is_none(py))
    } else {
        Ok(true)
    }
}
```

**Python-Migration** (`logprep/util/helper.py`):

```python
from logprep._rust import has_dotted_field  # noqa: F401
```

**Verifizierung:**
```bash
cargo test -p logprep-core
uv run pytest tests/unit/util/test_helper.py -vvv
```

**Performance-Test:**
```bash
uv run python ./benchmarks/benchmark_helpers.py --filter has_dotted_field \
  --baseline benchmarks/phase1d.json --output benchmarks/phase1e.json
```

---

### Schritt 1f: Pop-Operationen in Rust

**Ziel**: Alle Pop-Funktionen migrieren — `_pop_field_value`, `_pop_field_value_and_drop_empty`, `pop_dotted_field_value`.

**Rust-Implementierung** (`crates/logprep-core/src/field.rs`):

```rust
use pyo3::types::PyDict;

// --- _pop_field_value (intern) ---
fn pop_field_value<'py>(
    py: Python<'py>,
    event: &Bound<'py, pyo3::PyAny>,
    dotted_field: &str,
) -> PyResult<PyObject> {
    let parts = get_dotted_field_list(dotted_field);
    if parts.is_empty() {
        return Ok(py.None());
    }

    let parent_fields = &parts[..parts.len() - 1];
    let last_key = &parts[parts.len() - 1];

    let parent = if parent_fields.is_empty() {
        event.clone()
    } else {
        let mut current = event.clone();
        for part in parent_fields {
            current = get_item(py, &current, part)?;
        }
        current
    };

    if let Ok(dict) = parent.downcast::<PyDict>() {
        match dict.get_item(last_key)? {
            Some(value) => {
                dict.del_item(last_key)?;
                Ok(value.unbind())
            }
            None => Ok(py.None()),
        }
    } else {
        Ok(py.None())
    }
}

// --- _pop_field_value_and_drop_empty (intern) ---
fn pop_field_value_and_drop_empty<'py>(
    py: Python<'py>,
    event: &Bound<'py, pyo3::PyAny>,
    parts: &[String],
) -> PyResult<PyObject> {
    if parts.is_empty() {
        return Ok(py.None());
    }

    let next_key = &parts[0];
    let remaining = &parts[1..];

    if let Ok(dict) = event.downcast::<PyDict>() {
        match dict.get_item(next_key)? {
            Some(child) => {
                if remaining.is_empty() {
                    // Letzter Key — Wert entfernen und zurückgeben
                    let value = child.unbind();
                    dict.del_item(next_key)?;
                    return Ok(value);
                }

                // Rekursiv weiter
                let value = pop_field_value_and_drop_empty(py, &child, remaining)?;

                // Wenn Kind leer ist, auch entfernen
                if let Ok(child_dict) = child.downcast::<PyDict>() {
                    if child_dict.is_empty() {
                        dict.del_item(next_key)?;
                    }
                }

                Ok(value)
            }
            None => Ok(py.None()),
        }
    } else {
        Ok(py.None())
    }
}

// --- pop_dotted_field_value (exportiert) ---
#[pyfunction]
#[pyo3(signature = (event, dotted_field, drop_empty=true))]
fn pop_dotted_field_value(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    dotted_field: &str,
    drop_empty: bool,
) -> PyResult<PyObject> {
    let parts = get_dotted_field_list(dotted_field);

    if drop_empty {
        pop_field_value_and_drop_empty(py, event, &parts)
    } else {
        pop_field_value(py, event, dotted_field)
    }
}
```

**Python-Migration** (`logprep/util/helper.py`):

```python
from logprep._rust import pop_dotted_field_value as _pop_rust

def pop_dotted_field_value(event, dotted_field, drop_empty=True):
    result = _pop_rust(event, dotted_field, drop_empty)
    return MISSING if result is None else result
```

**Verifizierung:**
```bash
cargo test -p logprep-core
uv run pytest tests/unit/util/test_helper.py::TestPopDottedFieldValue -vvv
```

**Performance-Test:**
```bash
uv run python ./benchmarks/benchmark_helpers.py --filter pop_dotted_field_value \
  --baseline benchmarks/phase1e.json --output benchmarks/phase1f.json
```

---

### Schritt 1g: Write-Operationen in Rust

**Ziel**: Die gesamte Add/Write-Logik migrieren — `_add_and_overwrite_key`, `_add_and_not_overwrite_key`, `_add_field_to`, `_add_field_to_silent_fail`, `add_fields_to`.

**Rust-Implementierung** (`crates/logprep-core/src/field.rs`):

```rust
use pyo3::exceptions::PyException;

pyo3::create_exception!(logprep_core, FieldExistsWarning, PyException);

// --- _add_and_overwrite_key (intern) ---
fn add_and_overwrite_key<'py>(
    py: Python<'py>,
    event: &Bound<'py, pyo3::PyAny>,
    key: &str,
) -> PyResult<Bound<'py, pyo3::PyAny>> {
    if let Ok(dict) = event.downcast::<PyDict>() {
        if let Ok(Some(existing)) = dict.get_item(key) {
            if existing.downcast::<PyDict>().is_ok() {
                return Ok(existing);
            }
        }
        let sub_dict = PyDict::new(py);
        dict.set_item(key, &sub_dict)?;
        return Ok(sub_dict.into_any());
    }
    Err(pyo3::exceptions::PyTypeError::new_err("Container is not a dict"))
}

// --- _add_and_not_overwrite_key (intern) ---
fn add_and_not_overwrite_key<'py>(
    py: Python<'py>,
    event: &Bound<'py, pyo3::PyAny>,
    key: &str,
) -> PyResult<Bound<'py, pyo3::PyAny>> {
    if let Ok(dict) = event.downcast::<PyDict>() {
        if let Ok(Some(existing)) = dict.get_item(key) {
            if existing.downcast::<PyDict>().is_ok() {
                return Ok(existing);
            }
            return Err(pyo3::exceptions::PyKeyError::new_err("key exists"));
        }
        let sub_dict = PyDict::new(py);
        dict.set_item(key, &sub_dict)?;
        return Ok(sub_dict.into_any());
    }
    Err(pyo3::exceptions::PyTypeError::new_err("Container is not a dict"))
}

// --- _add_field_to (intern) ---
fn add_field_to(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    field_name: &str,
    content: &Bound<'_, pyo3::PyAny>,
    rule: Option<&Bound<'_, pyo3::PyAny>>,
    merge_with_target: bool,
    overwrite_target: bool,
) -> PyResult<()> {
    if merge_with_target && overwrite_target {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "Can't merge with and overwrite a target field at the same time",
        ));
    }

    let parts = get_dotted_field_list(field_name);
    if parts.is_empty() {
        return Err(pyo3::exceptions::PyValueError::new_err("Empty field path"));
    }
    let target_key = &parts[parts.len() - 1];

    if overwrite_target {
        let mut current = event.clone();
        for part in &parts[..parts.len() - 1] {
            current = add_and_overwrite_key(py, &current, part)?;
        }
        if let Ok(dict) = current.downcast::<PyDict>() {
            // Deep copy via Python's copy.deepcopy
            let copy_mod = py.import("copy")?;
            let deep_copy = copy_mod.getattr("deepcopy")?;
            let copied = deep_copy.call1((content,))?;
            dict.set_item(target_key, copied)?;
        }
        return Ok(());
    }

    // Kein Overwrite — prüfe ob Feld bereits existiert
    let mut current = event.clone();
    for part in &parts[..parts.len() - 1] {
        current = match add_and_not_overwrite_key(py, &current, part) {
            Ok(val) => val,
            Err(_) => {
                let warning = FieldExistsWarning::new_err((
                    rule.map(|r| r.clone_ref(py)).unwrap_or_else(|| py.None()),
                    event.clone_ref(py),
                    vec![pyo3::types::PyString::new(py, field_name).into_any()],
                ));
                return Err(warning);
            }
        };
    }

    if let Ok(dict) = current.downcast::<PyDict>() {
        let existing = dict.get_item(target_key)?;

        match existing {
            Some(existing_val) if existing_val.is_none(py) => {
                // None → überschreiben mit neuem Wert
                let copy_mod = py.import("copy")?;
                let deep_copy = copy_mod.getattr("deepcopy")?;
                dict.set_item(target_key, deep_copy.call1((content,))?)?;
            }
            None => {
                // Feld existiert nicht → einfach setzen
                let copy_mod = py.import("copy")?;
                let deep_copy = copy_mod.getattr("deepcopy")?;
                dict.set_item(target_key, deep_copy.call1((content,))?)?;
            }
            Some(existing_val) => {
                if !merge_with_target {
                    let warning = FieldExistsWarning::new_err((
                        rule.map(|r| r.clone_ref(py)).unwrap_or_else(|| py.None()),
                        event.clone_ref(py),
                        vec![pyo3::types::PyString::new(py, field_name).into_any()],
                    ));
                    return Err(warning);
                }

                // Merge-Logik
                if let (Ok(existing_dict), Ok(content_dict)) = (
                    existing_val.downcast::<PyDict>(),
                    content.downcast::<PyDict>(),
                ) {
                    existing_dict.update(content_dict)?;
                } else if let (Ok(existing_list), Ok(content_list)) = (
                    existing_val.downcast::<PyList>(),
                    content.downcast::<PyList>(),
                ) {
                    for item in content_list {
                        existing_list.append(item)?;
                    }
                } else if let Ok(existing_list) = existing_val.downcast::<PyList>() {
                    existing_list.append(content)?;
                } else if let Ok(content_list) = content.downcast::<PyList>() {
                    let new_list = PyList::empty(py);
                    new_list.append(existing_val)?;
                    for item in content_list {
                        new_list.append(item)?;
                    }
                    dict.set_item(target_key, new_list)?;
                } else {
                    if !overwrite_target {
                        let warning = FieldExistsWarning::new_err((
                            rule.map(|r| r.clone_ref(py)).unwrap_or_else(|| py.None()),
                            event.clone_ref(py),
                            vec![pyo3::types::PyString::new(py, field_name).into_any()],
                        ));
                        return Err(warning);
                    }
                    let new_list = PyList::empty(py);
                    new_list.append(existing_val)?;
                    let copy_mod = py.import("copy")?;
                    let deep_copy = copy_mod.getattr("deepcopy")?;
                    new_list.append(deep_copy.call1((content,))?)?;
                    dict.set_item(target_key, new_list)?;
                }
            }
        }
    }

    Ok(())
}

// --- _add_field_to_silent_fail (intern) ---
fn add_field_to_silent_fail(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    field_name: &str,
    content: &Bound<'_, pyo3::PyAny>,
    rule: Option<&Bound<'_, pyo3::PyAny>>,
    merge_with_target: bool,
    overwrite_target: bool,
) -> PyResult<Option<String>> {
    match add_field_to(py, event, field_name, content, rule, merge_with_target, overwrite_target) {
        Ok(()) => Ok(None),
        Err(e) => {
            if e.is_instance_of::<FieldExistsWarning>(py) {
                // Extrahiere skipped_fields[0] aus der Exception
                let args = e.value(py).getattr("skipped_fields")?;
                let first = args.get_item(0)?;
                Ok(Some(first.to_string()))
            } else {
                Err(e)
            }
        }
    }
}

// --- add_fields_to (exportiert) ---
#[pyfunction]
#[pyo3(signature = (event, fields, rule=None, merge_with_target=false, overwrite_target=false, skip_none=true))]
fn add_fields_to(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    fields: &Bound<'_, pyo3::PyDict>,
    rule: Option<&Bound<'_, pyo3::PyAny>>,
    merge_with_target: bool,
    overwrite_target: bool,
    skip_none: bool,
) -> PyResult<()> {
    // None-Werte filtern
    let filtered: Vec<(String, PyObject)> = fields
        .iter()
        .filter(|(k, v)| !skip_none || !v.is_none(py))
        .map(|(k, v)| (k.to_string(), v.unbind()))
        .collect();

    let num_fields = filtered.len();

    if num_fields == 1 {
        let (field_name, value) = &filtered[0];
        let value_bound = value.bind(py);
        add_field_to(py, event, field_name, value_bound, rule, merge_with_target, overwrite_target)?;
        return Ok(());
    }

    let mut unsuccessful = Vec::new();
    for (field_name, value) in &filtered {
        let value_bound = value.bind(py);
        if let Ok(Some(skipped)) = add_field_to_silent_fail(
            py, event, field_name, value_bound, rule, merge_with_target, overwrite_target,
        ) {
            unsuccessful.push(skipped);
        }
    }

    if !unsuccessful.is_empty() {
        let warning = FieldExistsWarning::new_err((
            rule.map(|r| r.clone_ref(py)).unwrap_or_else(|| py.None()),
            event.clone_ref(py),
            unsuccessful.into_pyobject(py)?,
        ));
        return Err(warning);
    }

    Ok(())
}
```

**Python-Migration** (`logprep/util/helper.py`):

```python
from logprep._rust import add_fields_to, FieldExistsWarning  # noqa: F401
```

Die internen Python-Funktionen `_add_field_to`, `_add_field_to_silent_fail`, `_add_and_overwrite_key`, `_add_and_not_overwrite_key`, `_get_slice_arg`, `_get_item` werden komplett aus `helper.py` entfernt.

**Verifizierung:**
```bash
cargo test -p logprep-core
uv run pytest tests/unit/util/test_helper_add_field.py -vvv
uv run pytest tests/unit/util/test_helper.py -vvv
```

**Performance-Test:**
```bash
uv run python ./benchmarks/benchmark_helpers.py --filter add_fields_to \
  --baseline benchmarks/phase1f.json --output benchmarks/phase1g.json
```

---

### Schritt 1h: Thin Wrapper in Python

**Ziel**: Die verbleibenden Python-Wrapper auf 1-3 Zeilen reduzieren. Die gesamte `helper.py` wird aufgeräumt.

**Python-Migration** (`logprep/util/helper.py`):

```python
# --- Thin Wrapper (bleiben in Python, nutzen Rust-Funktionen) ---

append_as_list = partial(add_fields_to, merge_with_target=True)


def add_and_overwrite(event, fields, rule, *_):
    """Wrapper für add_fields_to mit overwrite_target=True."""
    add_fields_to(event, fields, rule, overwrite_target=True)


def append(event, field, separator, rule):
    """Feld anhängen — Separator-basiert oder als Liste."""
    target_field, content = list(field.items())[0]
    target_value = get_dotted_field_value(event, target_field)
    if not isinstance(target_value, list):
        target_value = "" if target_value is None else target_value
        target_value = f"{target_value}{separator}{content}"
        add_and_overwrite(event, fields={target_field: target_value}, rule=rule)
    else:
        append_as_list(event, field)


def get_source_fields_dict(event, rule):
    """Dict mit Dotted-Fields als Keys und Werten aus dem Event."""
    source_fields = rule.source_fields
    return {field: get_dotted_field_value(event, field) for field in source_fields}


def copy_fields_to_event(
    target_event, source_event, dotted_field_names, *,
    skip_missing=True, merge_with_target=False, overwrite_target=False, rule=None,
):
    """Felder von source_event nach target_event kopieren."""
    on_missing_result = SKIP if skip_missing else None
    source_fields = get_dotted_field_values(
        source_event, dotted_field_names, on_missing=lambda _: on_missing_result
    )
    add_fields_to(
        target_event, source_fields, rule=rule,
        overwrite_target=overwrite_target, merge_with_target=merge_with_target,
        skip_none=False,
    )
```

**Was wird aus helper.py entfernt:**
- `_get_slice_arg` (in Rust)
- `_get_item` (in Rust)
- `get_dotted_field_list` (in Rust, kein `lru_cache`)
- `field_list_to_dotted_field` (in Rust)
- `join_dotted_fields` (in Rust)
- `get_dotted_field_value` (in Rust)
- `get_dotted_field_value_with_missing` (in Rust)
- `get_field_value` (in Rust)
- `get_field_value_no_slice` (in Rust)
- `get_dotted_field_values` (in Rust)
- `has_dotted_field` (in Rust)
- `pop_dotted_field_value` (in Rust, Wrapper mit MISSING-Mapping)
- `_pop_field_value` (in Rust)
- `_pop_field_value_and_drop_empty` (in Rust)
- `_add_and_overwrite_key` (in Rust)
- `_add_and_not_overwrite_key` (in Rust)
- `_add_field_to` (in Rust)
- `_add_field_to_silent_fail` (in Rust)
- `add_fields_to` (in Rust)

**Was bleibt in helper.py:**
- `Missing`, `MISSING`, `Skip`, `SKIP` (Sentinels)
- `DottedTemplate`
- `FieldValue`, `FieldRef`, `T` (Typ-Aliase)
- `append_as_list` (partial)
- `add_and_overwrite` (thin wrapper)
- `append` (thin wrapper)
- `get_source_fields_dict` (thin wrapper)
- `copy_fields_to_event` (thin wrapper)
- `recursive_compare`
- `remove_file_if_exists`
- `camel_to_snake`, `snake_to_camel`
- `get_versions_string`
- `deduplicate_with_order`
- `resolve_template`, `create_template_resolver`
- `reduce_field_value`, `transform_field_value`

**Verifizierung:**
```bash
cargo test -p logprep-core
uv run pytest tests/unit/util/test_helper.py -vvv
uv run pytest tests/unit/util/test_helper_add_field.py -vvv
uv run pytest ./tests --cov=logprep --cov-report=xml -vvv
pre-commit run --all-files
```

**Performance-Test:**
```bash
uv run python ./benchmarks/benchmark_helpers.py \
  --output benchmarks/phase1h.json
```

---

### Zusammenfassung: Reihenfolge der Commits

| # | Beschreibung | Betrifft | Risiko |
|---|---|---|---|
| 1a | Rust scaffolding (Cargo.toml, lib.rs, field.rs) | Nur neue Dateien | Minimal |
| 1b | maturin + nix update | pyproject.toml, flake.nix, uv.lock | Hoch — Build-Backend-Wechsel |
| 1c | Parsing → Rust: `get_dotted_field_list`, `field_list_to_dotted_field`, `join_dotted_fields` | helper.py, _rust/__init__.py | Niedrig — isolierte Funktionen |
| 1d | Read → Rust: `_get_item`, `get_dotted_field_value`, `get_dotted_field_value_with_missing`, `get_field_value`, `get_field_value_no_slice`, `get_dotted_field_values` | helper.py | Niedrig — Kern-Funktionen |
| 1e | Existenz → Rust: `has_dotted_field` | helper.py | Minimal — Wrapper |
| 1f | Pop → Rust: `_pop_field_value`, `_pop_field_value_and_drop_empty`, `pop_dotted_field_value` | helper.py | Mittel — Cleanup-Logik |
| 1g | Write → Rust: `_add_and_*_key`, `_add_field_to`, `_add_field_to_silent_fail`, `add_fields_to`, `FieldExistsWarning` | helper.py | Hoch — komplexeste Funktion |
| 1h | Aufräumen: Python-Wrapper, interne Funktionen entfernen | helper.py | Niedrig — nur Aufräumen |

**Jeder Commit** muss:
1. Alle bestehenden Tests bestehen (`uv run pytest ./tests -vvv`)
2. `pre-commit run --all-files` bestehen
3. `cargo test -p logprep-core` bestehen (ab Schritt 1c)
4. Nix-Docker-Image bauen (`nix build .#packages.x86_64-linux.docker.python312`)
5. CHANGELOG.md aktualisiert sein
6. Performance-Test durchführen (`./benchmarks`)

### Container-Build-Verifikation (nach jedem Commit)

```bash
nix build .#packages.x86_64-linux.docker.python312 -o image
docker load < image
docker run --rm logprep:py312 logprep --help
docker run --rm logprep:py312 python -c "from logprep._rust import get_dotted_field_list; print('OK')"
```

---

## Phase 2: Filter-Engine (FilterExpression AST)

**Ziel**: Filter-Expression AST + Lucene-Parser komplett in Rust implementieren, mit Python-Brücke für `logprep/ng/`.

**Begründung**: Reine Logik, Hot Path für jedes Rule-Matching, keine externen I/O-Abhängigkeiten. Verfügbare Rust-Crates für Pattern-Matching (regex, glob) werden genutzt statt Python-Bibliotheken.

### Rust-Struktur

```
crates/logprep-core/src/filter/
├── mod.rs
├── expression.rs    # FilterExpression Enum + Match-Logik
├── lucene.rs        # Lucene-Query-Parser
└── range.rs         # Range-Typen
```

**Enthält**: `StringFilterExpression`, `WildcardFilterExpression`, `SigmaFilterExpression`,
`IntegerFilterExpression`, `FloatFilterExpression`, `RangeExpression`, `RegExFilterExpression`,
`Exists`, `Null`, `And`, `Or`, `Not`

### Python-Brücke

```python
# logprep/ng/filter/expression.py
from logprep._rust.filter import (
    StringFilterExpression,
    WildcardFilterExpression,
    SigmaFilterExpression,
    IntegerFilterExpression,
    FloatFilterExpression,
    RangeExpression,
    RegExFilterExpression,
    Exists,
    Null,
    And,
    Or,
    Not,
)
```

Der gesamte `logprep/filter/` Python-Code wird durch die Rust-Implementierung ersetzt. Python-Dateien werden gelöscht oder auf Import-Wrapper reduziert.

### Verifizierung

```bash
cargo test -p logprep-core
uv run pytest tests/unit/filter/ -vvv
```

### Performance-Test

```bash
uv run python ./benchmarks/benchmark_filter.py --baseline benchmarks/phase1g.json \
  --output benchmarks/phase2.json
```

---

## Phase 3: Rule Tree + Rule Matching

**Ziel**: `RuleTree` und `Rule`-Matching komplett in Rust implementieren.

**Begründung**: Zentraler Matching-Mechanismus, wird von jedem `Processor.process()` aufgerufen. Nutzt die Rust-Filter-Engine aus Phase 2.

### Rust-Struktur

```
crates/logprep-core/src/rule/
├── mod.rs
├── tree.rs           # RuleTree (Baum-Struktur)
├── segment.rs        # Rule-Segmentierung
└── matcher.rs        # Matching-Logik
```

### Python-Brücke

```python
# logprep/ng/framework/rule_tree/
from logprep._rust.rule import RuleTree, Rule
```

Der gesamte `logprep/ng/framework/rule_tree/` Python-Code wird durch die Rust-Implementierung ersetzt.

### Verifizierung

```bash
cargo test -p logprep-core
uv run pytest tests/unit/framework/rule_tree/ -vvv
```

### Performance-Test

```bash
uv run python ./benchmarks/benchmark_rule_matching.py --baseline benchmarks/phase2.json \
  --output benchmarks/phase3.json
```

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
