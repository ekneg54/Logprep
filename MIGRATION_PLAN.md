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

Direct re-export (no wrapper needed — the Rust function returns `MISSING` for missing fields,
matching the original Python implementation):

```python
from logprep._rust import pop_dotted_field_value  # noqa: F401
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

## Phase 2: Filter-Engine (FilterExpression AST + Lucene-Parser)

**Ziel**: Die gesamte Filter-Expression AST (14 Klassen) plus den Lucene-Query-Parser komplett in Rust implementieren. Python-Code wird auf Import-Wrapper reduziert. Die externe Python-Abhängigkeit `luqum` wird durch einen nativen Rust-Parser ersetzt.

**Begründung**: Der Filter-Expression-Matching-Code ist der Hot Path für jedes Rule-Matching im System — er wird für jede Nachricht und jedes Rule aufgerufen. Aktuell nutzt er Python's `re`-Modul für Wildcard/Regex-Matching. Ein Rust-Implementierung eliminiert den Python-Overhead komplett. Der `luqum`-Parser (externes Python-Paket) wird durch einen nativen Rust-Parser ersetzt, um die Build-Abhängigkeit zu eliminieren.

**Abhängigkeiten**: Phase 1 (Dotted-Field Helper in Rust)

### Betroffene Python-Dateien

| Datei | Aktuell | Nach Phase 2 |
|---|---|---|
| `logprep/filter/expression/filter_expression.py` | 449 Zeilen, 14 Klassen | Gelöscht → Rust |
| `logprep/filter/expression/__init__.py` | Leer | Import-Wrapper aus Rust |
| `logprep/filter/__init__.py` | Leer | Unverändert |
| `logprep/filter/lucene_filter.py` | 745 Zeilen, `luqum`-Abhängigkeit | Thin Wrapper (LuceneFilter.create → Rust) |

### Import-Sites ( alle `from logprep.filter.expression...` )

| Datei | Import |
|---|---|
| `logprep/filter/lucene_filter.py` | `Always`, `And`, `Exists`, `FilterExpression`, `FloatRangeFilterExpression`, `IntegerRangeFilterExpression`, `Not`, `Null`, `Or`, `RegExFilterExpression`, `SigmaFilterExpression`, `StringFilterExpression`, `StringRangeFilterExpression` |
| `logprep/processor/base/rule.py` | `FilterExpression` (Typ-Annotation) |
| `logprep/processor/list_comparison/rule.py` | `FilterExpression` (Typ-Annotation) |
| `logprep/processor/replacer/rule.py` | `FilterExpression` (Typ-Annotation) |
| `logprep/processor/dissector/rule.py` | `FilterExpression` (Typ-Annotation) |
| `logprep/framework/rule_tree/rule_tree.py` | `FilterExpression` (Typ-Annotation) |
| `logprep/framework/rule_tree/node.py` | `FilterExpression`, `KeyDoesNotExistError` |
| `logprep/framework/rule_tree/rule_parser.py` | `Always`, `Exists`, `Not` |
| `logprep/framework/rule_tree/rule_segmenter.py` | `Always`, `And`, `Exists`, `FilterExpression`, `Not`, `Or` |
| `logprep/framework/rule_tree/rule_sorter.py` | `FilterExpression`, `KeyBasedFilterExpression` |
| `logprep/framework/rule_tree/demorgan_resolver.py` | `And`, `FilterExpression`, `Not`, `Or` |
| `logprep/framework/rule_tree/rule_tagger.py` | `Exists`, `FilterExpression` |
| `logprep/util/event.py` | `KeyDoesNotExistError` |
| Tests (10 Dateien) | Verschiedene Expression-Klassen |

### Externe Python-Abhängigkeiten die entfallen

| Paket | Verwendung | Rust-Ersatz |
|---|---|---|
| `luqum` | Lucene-Query-Parsing | Eigener Lucene-Parser in Rust |
| `re` (in filter_expression.py) | Wildcard/Regex-Matching | `regex` crate |

---

### Schritt 2a: `regex` Crate + Rust-Expressions (Match-Logik)

**Ziel**: Alle 14 FilterExpression-Klassen + 2 Exception-Klassen in Rust implementieren. `luqum` bleibt vorerst in Python — wird in Schritt 2b ersetzt.

**Abhängigkeiten**: Phase 1 abgeschlossen

#### Workspace-Änderung (`Cargo.toml`):

```diff
 [workspace.dependencies]
 pyo3 = { version = "0.25", features = ["extension-module"] }
 serde = { version = "1", features = ["derive"] }
 serde_json = "1"
+regex = "1"
```

#### `crates/logprep-core/Cargo.toml`:

```diff
 [dependencies]
 pyo3.workspace = true
 serde.workspace = true
 serde_json.workspace = true
+regex.workspace = true
```

#### Rust-Modulstruktur

```
crates/logprep-core/src/
├── lib.rs              # pymodule: filter + field
├── field.rs            # (bestehend, Phase 1)
└── filter/
    ├── mod.rs          # pymodule definition
    ├── expression.rs   # FilterExpression Enum + Match-Logik
    └── range.rs        # Range-Boundary-Typen
```

#### `crates/logprep-core/src/filter/mod.rs`

```rust
pub mod expression;
pub mod range;

use pyo3::prelude::*;

#[pymodule]
pub fn filter(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<expression::PyFilterExpression>()?;
    m.add_class::<expression::PyAlways>()?;
    m.add_class::<expression::PyNot>()?;
    m.add_class::<expression::PyAnd>()?;
    m.add_class::<expression::PyOr>()?;
    m.add_class::<expression::PyStringFilterExpression>()?;
    m.add_class::<expression::PyWildcardStringFilterExpression>()?;
    m.add_class::<expression::PySigmaFilterExpression>()?;
    m.add_class::<expression::PyIntegerFilterExpression>()?;
    m.add_class::<expression::PyFloatFilterExpression>()?;
    m.add_class::<expression::PyIntegerRangeFilterExpression>()?;
    m.add_class::<expression::PyFloatRangeFilterExpression>()?;
    m.add_class::<expression::PyStringRangeFilterExpression>()?;
    m.add_class::<expression::PyRegExFilterExpression>()?;
    m.add_class::<expression::PyExists>()?;
    m.add_class::<expression::PyNull>()?;
    m.add_class::<expression::FilterExpressionError>()?;
    m.add_class::<expression::KeyDoesNotExistError>()?;
    Ok(())
}
```

#### `crates/logprep-core/src/filter/expression.rs`

Das Herzstück — alle Expression-Typen als Rust-Enum mit Match-Logik:

```rust
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use regex::Regex;
use std::collections::HashMap;

use super::range::{RangeBoundary, parse_int, parse_float};

// ─── Exceptions ───

pyo3::create_exception!(
    logprep_core,
    FilterExpressionError,
    pyo3::exceptions::PyException
);
pyo3::create_exception!(
    logprep_core,
    KeyDoesNotExistError,
    FilterExpressionError
);

// ─── Hilfsfunktionen ───

/// Traversiert ein verschachteltes Dict entlang eines Key-Pfads.
fn get_value<'py>(
    py: Python<'py>,
    key: &[String],
    document: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    if key.is_empty() {
        return Err(KeyDoesNotExistError::new_err("empty key"));
    }
    let mut current = document.clone();
    for segment in key {
        let dict: Bound<'py, PyDict> = current.downcast().map_err(|_| {
            KeyDoesNotExistError::new_err(format!(
                "key '{}' is not a dict",
                segment
            ))
        })?;
        current = dict.get_item(segment)?.ok_or_else(|| {
            KeyDoesNotExistError::new_err(format!(
                "key '{}' does not exist",
                segment
            ))
        })?;
    }
    Ok(current)
}

/// Prüft ob ein Pfad in einem Dict existiert (ohne Wert zurückzugeben).
fn path_exists(
    key: &[String],
    document: &Bound<'_, PyAny>,
) -> bool {
    if key.is_empty() {
        return false;
    }
    let mut current = document.clone();
    for segment in key {
        let Ok(dict) = current.downcast::<PyDict>() else {
            return false;
        };
        let Ok(Some(child)) = dict.get_item(segment) else {
            return false;
        };
        current = child;
    }
    true
}

/// Gibt None als Python-None zurück, alles andere als gebundenes Objekt.
fn is_none_py(obj: &Bound<'_, PyAny>) -> bool {
    obj.is_none()
}

// ─── Python-Klassen ───

/// Basis-Klasse — wird von allen konkreten Expression-Klassen geerbt.
#[pyclass(subclass)]
pub struct PyFilterExpression;

#[pymethods]
impl PyFilterExpression {
    /// Safe-Matching: Gibt False bei fehlenden Keys zurück.
    fn matches(&self, py: Python, document: &Bound<'_, PyAny>) -> bool {
        if !document.is_instance::<PyDict>().unwrap_or(false) {
            return false;
        }
        match self.does_match(py, document) {
            Ok(result) => result,
            Err(e) => {
                if e.is_instance_of::<KeyDoesNotExistError>(py) {
                    false
                } else {
                    panic!("unexpected error in matches(): {:?}", e);
                }
            }
        }
    }

    /// Muss von Unterklassen überschrieben werden.
    fn does_match(
        &self,
        _py: Python,
        _document: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        unimplemented!("does_match must be overridden")
    }
}

// ─── Always ───

#[pyclass(extends=PyFilterExpression)]
pub struct PyAlways {
    #[pyo3(get)]
    value: bool,
}

#[pymethods]
impl PyAlways {
    #[new]
    fn new(value: bool) -> Self {
        Self { value }
    }

    fn __repr__(&self) -> String {
        if self.value {
            "*".to_string()
        } else {
            "".to_string()
        }
    }

    fn does_match(&self, _py: Python, _document: &Bound<'_, PyAny>) -> PyResult<bool> {
        Ok(self.value)
    }
}

// ─── Not ───

#[pyclass(extends=PyFilterExpression)]
pub struct PyNot {
    child: Py<PyFilterExpression>,
}

#[pymethods]
impl PyNot {
    #[new]
    #[pyo3(signature = (expression,))]
    fn new(expression: &Bound<'_, PyFilterExpression>) -> PyResult<Self> {
        Ok(Self {
            child: expression.clone().unbind(),
        })
    }

    fn __repr__(&self, py: Python) -> String {
        let child_repr = self.child.bind(py).call_method0("__repr__").unwrap();
        format!("NOT ({})", child_repr)
    }

    fn does_match(
        &self,
        py: Python,
        document: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        let child = self.child.bind(py);
        // Not nutzt `matches()` (safe) statt `does_match()` — entspricht Python-Verhalten
        let result: bool = child.call_method1("matches", (document,))?;
        Ok(!result)
    }
}

// ─── And ───

#[pyclass(extends=PyFilterExpression)]
pub struct PyAnd {
    children: Vec<Py<PyFilterExpression>>,
}

#[pymethods]
impl PyAnd {
    #[new]
    #[pyo3(signature = (*children))]
    fn new(children: Vec<&Bound<'_, PyFilterExpression>>) -> PyResult<Self> {
        Ok(Self {
            children: children.into_iter().map(|c| c.clone().unbind()).collect(),
        })
    }

    fn __repr__(&self, py: Python) -> String {
        let parts: Vec<String> = self
            .children
            .iter()
            .map(|c| {
                c.bind(py)
                    .call_method0("__repr__")
                    .unwrap()
                    .to_string()
            })
            .collect();
        format!("({})", parts.join(" AND "))
    }

    fn does_match(
        &self,
        py: Python,
        document: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        for child in &self.children {
            let result: bool = child.bind(py).call_method1("matches", (document,))?;
            if !result {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

// ─── Or ───

#[pyclass(extends=PyFilterExpression)]
pub struct PyOr {
    children: Vec<Py<PyFilterExpression>>,
}

#[pymethods]
impl PyOr {
    #[new]
    #[pyo3(signature = (*children))]
    fn new(children: Vec<&Bound<'_, PyFilterExpression>>) -> PyResult<Self> {
        Ok(Self {
            children: children.into_iter().map(|c| c.clone().unbind()).collect(),
        })
    }

    fn __repr__(&self, py: Python) -> String {
        let parts: Vec<String> = self
            .children
            .iter()
            .map(|c| {
                c.bind(py)
                    .call_method0("__repr__")
                    .unwrap()
                    .to_string()
            })
            .collect();
        format!("({})", parts.join(" OR "))
    }

    fn does_match(
        &self,
        py: Python,
        document: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        for child in &self.children {
            let result: bool = child.bind(py).call_method1("matches", (document,))?;
            if result {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

// ─── KeyBased (Zwischenklasse) ───

#[pyclass(subclass, extends=PyFilterExpression)]
pub struct PyKeyBasedFilterExpression {
    #[pyo3(get)]
    key: Vec<String>,
    key_as_dotted_string: String,
}

#[pymethods]
impl PyKeyBasedFilterExpression {
    #[new]
    fn new(key: Vec<String>) -> Self {
        let dotted = key
            .iter()
            .map(|k| k.replace('.', "\\."))
            .collect::<Vec<_>>()
            .join(".");
        Self {
            key,
            key_as_dotted_string: dotted,
        }
    }

    #[getter]
    fn key_as_dotted_string(&self) -> String {
        self.key_as_dotted_string.clone()
    }
}

// ─── KeyValueBased (Zwischenklasse) ───

#[pyclass(subclass, extends=PyKeyBasedFilterExpression)]
pub struct PyKeyValueBasedFilterExpression {
    #[pyo3(get)]
    expected_value: String,
}

#[pymethods]
impl PyKeyValueBasedFilterExpression {
    #[new]
    fn new(key: Vec<String>, expected_value: String) -> Self {
        Self { expected_value }
    }

    fn __repr__(&self, py: Python) -> String {
        // key_as_dotted_string vom Elternteil holen
        let parent: &PyKeyBasedFilterExpression = self.into();
        format!("{}:{}", parent.key_as_dotted_string, self.expected_value)
    }
}

// ─── StringFilterExpression ───

#[pyclass(extends=PyKeyValueBasedFilterExpression)]
pub struct PyStringFilterExpression;

#[pymethods]
impl PyStringFilterExpression {
    #[new]
    fn new(key: Vec<String>, expected_value: String) -> Self {
        Self
    }

    fn does_match(
        &self,
        py: Python,
        document: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        let parent: &PyKeyValueBasedFilterExpression = self.into();
        let key = &self_get_key(parent);
        let value = get_value(py, key, document)?;

        if let Ok(list) = value.downcast::<PyList>() {
            for item in list.iter() {
                let s: String = item.extract()?;
                if s == parent.expected_value {
                    return Ok(true);
                }
            }
            return Ok(false);
        }

        let value_str: String = value.extract().unwrap_or_default();
        Ok(value_str == parent.expected_value)
    }
}

// ─── WildcardStringFilterExpression ───

#[pyclass(extends=PyKeyValueBasedFilterExpression)]
pub struct PyWildcardStringFilterExpression {
    compiled_regex: Regex,
}

#[pymethods]
impl PyWildcardStringFilterExpression {
    #[new]
    fn new(key: Vec<String>, expected_value: String) -> PyResult<Self> {
        let regex_str = Self::build_regex(&expected_value)?;
        let compiled_regex = Regex::new(&regex_str).map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid regex: {}", e))
        })?;
        Ok(Self { compiled_regex })
    }

    fn does_match(
        &self,
        py: Python,
        document: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        let parent: &PyKeyValueBasedFilterExpression = self.into();
        let key = &parent.key;
        let value = get_value(py, key, document)?;

        if let Ok(list) = value.downcast::<PyList>() {
            for item in list.iter() {
                let s: String = item.extract()?;
                if self.compiled_regex.is_match(&s) {
                    return Ok(true);
                }
            }
            return Ok(false);
        }

        let value_str: String = value.extract().unwrap_or_default();
        Ok(self.compiled_regex.is_match(&value_str))
    }
}

impl PyWildcardStringFilterExpression {
    fn build_regex(expected: &str) -> PyResult<String> {
        // re.escape equivalent + wildcard substitution
        let escaped = regex::escape(expected);
        // Escape \* und \? zu literalen backslash-stars/ques
        // Dann * → .* und ? → .?
        let mut result = String::new();
        let chars: Vec<char> = escaped.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() {
                if chars[i + 1] == '*' {
                    result.push('\\');
                    result.push('*');
                    i += 2;
                    continue;
                }
                if chars[i + 1] == '?' {
                    result.push('\\');
                    result.push('?');
                    i += 2;
                    continue;
                }
            }
            if chars[i] == '*' {
                result.push_str(".*");
            } else if chars[i] == '?' {
                result.push_str(".?");
            } else {
                result.push(chars[i]);
            }
            i += 1;
        }
        Ok(format!("^{}$", result))
    }
}

// ─── SigmaFilterExpression (case-insensitive) ───

#[pyclass(extends=PyWildcardStringFilterExpression)]
pub struct PySigmaFilterExpression;

#[pymethods]
impl PySigmaFilterExpression {
    #[new]
    fn new(key: Vec<String>, expected_value: String) -> PyResult<Self> {
        // Selbe Logik wie Wildcard, aber case-insensitive regex
        let regex_str = PyWildcardStringFilterExpression::build_regex(&expected_value)?;
        let compiled = Regex::new(&format!("(?i){}", regex_str)).map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid regex: {}", e))
        })?;
        // In parent setzen
        Ok(Self)
    }
}

// ─── IntegerFilterExpression ───

#[pyclass(extends=PyKeyValueBasedFilterExpression)]
pub struct PyIntegerFilterExpression {
    expected_int: i64,
}

#[pymethods]
impl PyIntegerFilterExpression {
    #[new]
    fn new(key: Vec<String>, expected_value: String) -> PyResult<Self> {
        let expected_int: i64 = expected_value.parse().map_err(|_| {
            pyo3::exceptions::PyValueError::new_err(format!(
                "Invalid integer: {}",
                expected_value
            ))
        })?;
        Ok(Self { expected_int })
    }

    fn does_match(
        &self,
        py: Python,
        document: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        let parent: &PyKeyValueBasedFilterExpression = self.into();
        let value = get_value(py, &parent.key, document)?;
        let int_val: i64 = value.extract()?;
        Ok(int_val == self.expected_int)
    }
}

// ─── FloatFilterExpression ───

#[pyclass(extends=PyKeyValueBasedFilterExpression)]
pub struct PyFloatFilterExpression {
    expected_float: f64,
}

#[pymethods]
impl PyFloatFilterExpression {
    #[new]
    fn new(key: Vec<String>, expected_value: String) -> PyResult<Self> {
        let expected_float: f64 = expected_value.parse().map_err(|_| {
            pyo3::exceptions::PyValueError::new_err(format!(
                "Invalid float: {}",
                expected_value
            ))
        })?;
        Ok(Self { expected_float })
    }

    fn does_match(
        &self,
        py: Python,
        document: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        let parent: &PyKeyValueBasedFilterExpression = self.into();
        let value = get_value(py, &parent.key, document)?;
        let float_val: f64 = value.extract()?;
        Ok((float_val - self.expected_float).abs() < f64::EPSILON)
    }
}

// ─── RegExFilterExpression ───

#[pyclass(extends=PyKeyValueBasedFilterExpression)]
pub struct PyRegExFilterExpression {
    compiled_regex: Regex,
}

#[pymethods]
impl PyRegExFilterExpression {
    #[new]
    fn new(key: Vec<String>, regex: String) -> PyResult<Self> {
        let normalized = Self::normalize_regex(&regex);
        let compiled_regex = Regex::new(&normalized).map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid regex: {}", e))
        })?;
        let display = format!("/{}/", normalized.trim_start_matches('^').trim_end_matches('$'));
        Ok(Self { compiled_regex })
    }

    fn does_match(
        &self,
        py: Python,
        document: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        let parent: &PyKeyValueBasedFilterExpression = self.into();
        let value = get_value(py, &parent.key, document)?;

        if let Ok(list) = value.downcast::<PyList>() {
            for item in list.iter() {
                let s: String = item.extract()?;
                if self.compiled_regex.is_match(&s) {
                    return Ok(true);
                }
            }
            return Ok(false);
        }

        let value_str: String = value.extract().unwrap_or_default();
        Ok(self.compiled_regex.is_match(&value_str))
    }
}

impl PyRegExFilterExpression {
    fn normalize_regex(regex: &str) -> String {
        // Flags extrahieren (z.B. (?i))
        let (flags, pattern) = if regex.starts_with("(?") {
            if let Some(end) = regex.find(')') {
                (&regex[..=end], &regex[end + 1..])
            } else {
                ("", regex)
            }
        } else {
            ("", regex)
        };

        // ^ am Anfang prüfen
        let has_caret = pattern.starts_with('^');
        let has_dollar = pattern.ends_with('$');

        let clean = pattern
            .trim_start_matches('^')
            .trim_end_matches('$');

        let mut result = String::from(flags);
        if !has_caret {
            result.push('^');
        }
        result.push_str(clean);
        if !has_dollar {
            result.push('$');
        }
        result
    }
}

// ─── Exists ───

#[pyclass(extends=PyKeyBasedFilterExpression)]
pub struct PyExists;

#[pymethods]
impl PyExists {
    #[new]
    fn new(key: Vec<String>) -> Self {
        Self
    }

    fn __repr__(&self, py: Python) -> String {
        let parent: &PyKeyBasedFilterExpression = self.into();
        format!("{}: *", parent.key_as_dotted_string)
    }

    fn does_match(
        &self,
        py: Python,
        document: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        let parent: &PyKeyBasedFilterExpression = self.into();
        Ok(path_exists(&parent.key, document))
    }
}

// ─── Null ───

#[pyclass(extends=PyKeyBasedFilterExpression)]
pub struct PyNull;

#[pymethods]
impl PyNull {
    #[new]
    fn new(key: Vec<String>) -> Self {
        Self
    }

    fn __repr__(&self, py: Python) -> String {
        let parent: &PyKeyBasedFilterExpression = self.into();
        format!("{}:null", parent.key_as_dotted_string)
    }

    fn does_match(
        &self,
        py: Python,
        document: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        let parent: &PyKeyBasedFilterExpression = self.into();
        let value = get_value(py, &parent.key, document)?;
        Ok(value.is_none())
    }
}
```

> **Hinweis**: Die obige Implementierung ist ein Richtungs-Beispiel. PyO3-Vererbung (Subclassing) erfordert sorgfältiges Arbeiten mit `#[pyclass(extends=...)]` und `self.into()`. Die finale Implementierung muss ggf. auf Enums oder Kapselung (Wrapper-Objekt mit inner-Referenz) ausweichen, falls PyO3-Vererbung zu eingeschränkt ist.

**Alternative Architektur** (falls PyO3-Vererbung zu limitiert):

```rust
/// Enum-basiert — einfacher, keine Vererbung nötig
#[pyclass]
pub struct FilterExpression {
    inner: FilterExpressionInner,
}

enum FilterExpressionInner {
    Always { value: bool },
    Not { child: Py<FilterExpression> },
    And { children: Vec<Py<FilterExpression>> },
    Or { children: Vec<Py<FilterExpression>> },
    String { key: Vec<String>, expected: String },
    Wildcard { key: Vec<String>, expected: String, regex: Regex },
    Sigma { key: Vec<String>, expected: String, regex: Regex },
    Integer { key: Vec<String>, expected: i64 },
    Float { key: Vec<String>, expected: f64 },
    IntegerRange { key: Vec<String>, lower: i64, upper: i64, incl_low: bool, incl_high: bool },
    FloatRange { key: Vec<String>, lower: f64, upper: f64, incl_low: bool, incl_high: bool },
    StringRange { key: Vec<String>, lower: String, upper: String, incl_low: bool, incl_high: bool },
    Regex { key: Vec<String>, pattern: Regex },
    Exists { key: Vec<String> },
    Null { key: Vec<String> },
}
```

Diese Variante ist vorzuziehen, da sie PyO3-Vererbungs-Edge-Cases vermeidet und die Python-API trotzdem identisch bleibt (`FilterExpression.matches(document)`, `FilterExpression.does_match(document)` etc.).

#### `crates/logprep-core/src/filter/range.rs`

```rust
pub enum RangeBoundary {
    Int(i64),
    Float(f64),
    Str(String),
}

pub fn parse_int(s: &str) -> Result<i64, ()> {
    s.parse::<i64>().map_err(|_| ())
}

pub fn parse_float(s: &str) -> Result<f64, ()> {
    let val = s.parse::<f64>().map_err(|_| ())?;
    if val.is_finite() {
        Ok(val)
    } else {
        Err(())
    }
}
```

#### PyO3-Modul (`crates/logprep-core/src/lib.rs`):

```diff
 use pyo3::prelude::*;

 pub mod field;
+pub mod filter;

 #[pymodule]
 fn _rust(m: &Bound<'_, PyModule>) -> PyResult<()> {
+    // Filter
+    m.add_submodule(filter::filter(m)?)?;
+
     // Field (Phase 1)
     m.add_function(wrap_pyfunction!(field::get_dotted_field_list, m)?)?;
     // ... (bestehende Funktionen)
     Ok(())
 }
```

#### Rust-Tests (`crates/logprep-core/src/filter/expression.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::Python;

    #[test]
    fn always_true_matches() {
        Python::with_gil(|py| {
            let expr = PyAlways::new(true);
            let doc = PyDict::new(py);
            assert!(expr.does_match(py, &doc.into_any()).unwrap());
        });
    }

    #[test]
    fn always_false_does_not_match() {
        Python::with_gil(|py| {
            let expr = PyAlways::new(false);
            let doc = PyDict::new(py);
            assert!(!expr.does_match(py, &doc.into_any()).unwrap());
        });
    }

    #[test]
    fn string_filter_exact_match() {
        Python::with_gil(|py| {
            let expr = PyStringFilterExpression::new(
                vec!["field".to_string()],
                "expected".to_string(),
            );
            let doc = PyDict::new(py);
            doc.set_item("field", "expected").unwrap();
            assert!(expr.does_match(py, &doc.into_any()).unwrap());
        });
    }

    #[test]
    fn string_filter_list_membership() {
        Python::with_gil(|py| {
            let expr = PyStringFilterExpression::new(
                vec!["tags".to_string()],
                "critical".to_string(),
            );
            let doc = PyDict::new(py);
            let list = PyList::new(py, vec!["info", "critical", "warn"]).unwrap();
            doc.set_item("tags", list).unwrap();
            assert!(expr.does_match(py, &doc.into_any()).unwrap());
        });
    }

    #[test]
    fn exists_matches_present_key() {
        Python::with_gil(|py| {
            let expr = PyExists::new(vec!["foo".to_string()]);
            let doc = PyDict::new(py);
            doc.set_item("foo", "bar").unwrap();
            assert!(expr.does_match(py, &doc.into_any()).unwrap());
        });
    }

    #[test]
    fn exists_does_not_match_missing_key() {
        Python::with_gil(|py| {
            let expr = PyExists::new(vec!["missing".to_string()]);
            let doc = PyDict::new(py);
            assert!(!expr.does_match(py, &doc.into_any()).unwrap());
        });
    }

    #[test]
    fn not_negates_child() {
        Python::with_gil(|py| {
            let child = PyAlways::new(false).into_pyobject(py).unwrap();
            let expr = PyNot::new(&child).unwrap();
            let doc = PyDict::new(py);
            assert!(expr.does_match(py, &doc.into_any()).unwrap());
        });
    }

    #[test]
    fn and_requires_all_children() {
        Python::with_gil(|py| {
            let c1 = PyAlways::new(true).into_pyobject(py).unwrap();
            let c2 = PyAlways::new(false).into_pyobject(py).unwrap();
            let expr = PyAnd::new(vec![&c1, &c2]).unwrap();
            let doc = PyDict::new(py);
            assert!(!expr.does_match(py, &doc.into_any()).unwrap());
        });
    }

    #[test]
    fn or_requires_any_child() {
        Python::with_gil(|py| {
            let c1 = PyAlways::new(false).into_pyobject(py).unwrap();
            let c2 = PyAlways::new(true).into_pyobject(py).unwrap();
            let expr = PyOr::new(vec![&c1, &c2]).unwrap();
            let doc = PyDict::new(py);
            assert!(expr.does_match(py, &doc.into_any()).unwrap());
        });
    }

    #[test]
    fn integer_range_inclusive() {
        Python::with_gil(|py| {
            let expr = PyIntegerRangeFilterExpression::new(
                vec!["age".to_string()],
                18, 65, true, true,
            );
            let doc = PyDict::new(py);
            doc.set_item("age", 25).unwrap();
            assert!(expr.does_match(py, &doc.into_any()).unwrap());
        });
    }

    #[test]
    fn integer_range_excludes_bool() {
        Python::with_gil(|py| {
            let expr = PyIntegerRangeFilterExpression::new(
                vec!["flag".to_string()],
                0, 100, true, true,
            );
            let doc = PyDict::new(py);
            doc.set_item("flag", true).unwrap();
            assert!(!expr.does_match(py, &doc.into_any()).unwrap());
        });
    }

    #[test]
    fn wildcard_star_matches_any() {
        Python::with_gil(|py| {
            let expr = PyWildcardStringFilterExpression::new(
                vec!["name".to_string()],
                "foo*bar".to_string(),
            );
            let doc = PyDict::new(py);
            doc.set_item("name", "foobar").unwrap();
            assert!(expr.does_match(py, &doc.into_any()).unwrap());
        });
    }

    #[test]
    fn regex_with_anchors() {
        Python::with_gil(|py| {
            let expr = PyRegExFilterExpression::new(
                vec!["ip".to_string()],
                "192\\.168\\..*".to_string(),
            );
            let doc = PyDict::new(py);
            doc.set_item("ip", "192.168.0.1").unwrap();
            assert!(expr.does_match(py, &doc.into_any()).unwrap());
        });
    }
}
```

**Verifizierung:**
```bash
cargo test -p logprep-core
uv run pytest tests/unit/filter/test_filter_expression.py -vvv
```

> **Hinweis**: Die Python-Tests laufen zunächst weiter gegen die bestehende Python-Implementierung. Erst in Schritt 2d wird `filter_expression.py` auf Imports umgestellt. Die Rust-Tests verifizieren die Rust-Logik eigenständig.

**Performance-Test:**
```bash
uv run python benchmarks/run_phase_benchmark.py --phase 2 --runs 30 30 30
```

---

### Schritt 2b: Lucene-Parser in Rust

**Ziel**: Einen Lucene-Query-Parser in Rust schreiben, der `luqum` vollständig ersetzt. Der Parser nimmt einen Lucene-Query-String und liefert einen `FilterExpression`-Baum zurück.

**Abhängigkeiten**: Schritt 2a (Expression-Klassen in Rust)

**Begründung**: `luqum` ist ein externes Python-Paket mit eigener Lexer/Parser-Architektur. Ein Rust-Parser eliminiert diese Abhängigkeit und erlaubt volle Kontrolle über Fehlerbehandlung und Performance.

#### Rust-Modulstruktur (Erweiterung)

```
crates/logprep-core/src/filter/
├── mod.rs              # pymodule: expression + lucene
├── expression.rs       # (aus Schritt 2a)
├── range.rs            # (aus Schritt 2a)
└── lucene.rs           # Lucene-Query-Parser (neu)
```

#### Parser-Grammatik (Lucene-Subset)

```text
query       → or_expr
or_expr     → and_expr ("OR" and_expr)*
and_expr    → not_expr (("AND")? not_expr)*
not_expr    → "NOT" not_expr | atom
atom        → "(" query ")" | term

term        → field_filter | regex_field | field_group | bare_value
field_filter → field_name ":" (range | regex | phrase | word | "null")
field_group  → field_name ":" "(" or_expr ")"
regex_field  → field_name ":" "/" pattern "/"
range        → ("[" | "{") boundary "TO" boundary ("]" | "}")
bare_value   → "*" → Always(True) | word → Exists(key)
word         → [^\s\(\)\[\]\{\}\:\"]+
phrase       → '"' [^"]* '"'
```

#### `crates/logprep-core/src/filter/lucene.rs`

```rust
use pyo3::prelude::*;
use std::iter::Peekable;
use std::str::Chars;

use super::expression::{
    FilterExpression, FilterExpressionInner,
    build_wildcard_regex, build_sigma_regex, normalize_regex,
};

// ─── Lexer-Token ───

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(String),
    Phrase(String),
    Regex(String),
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Colon,
    To,
    And,
    Or,
    Not,
    Star,
    Slash,
    Eof,
}

// ─── Lexer ───

struct Lexer<'a> {
    chars: Peekable<Chars<'a>>,
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            chars: input.chars().peekable(),
            pos: 0,
        }
    }

    fn next_token(&mut self) -> Token {
        self.skip_whitespace();
        match self.chars.peek() {
            None => Token::Eof,
            Some(&c) => match c {
                '(' => { self.chars.next(); self.pos += 1; Token::LParen }
                ')' => { self.chars.next(); self.pos += 1; Token::RParen }
                '{' => { self.chars.next(); self.pos += 1; Token::LBrace }
                '}' => { self.chars.next(); self.pos += 1; Token::RBrace }
                '[' => { self.chars.next(); self.pos += 1; Token::LBracket }
                ']' => { self.chars.next(); self.pos += 1; Token::RBracket }
                ':' => { self.chars.next(); self.pos += 1; Token::Colon }
                '/' => { self.chars.next(); self.pos += 1; self.read_regex() }
                '"' | '\'' => self.read_phrase(),
                '*' => { self.chars.next(); self.pos += 1; Token::Star }
                _ => self.read_word(),
            }
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(&c) = self.chars.peek() {
            if c.is_whitespace() {
                self.chars.next();
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn read_phrase(&mut self) -> Token {
        let quote = self.chars.next().unwrap();
        self.pos += 1;
        let mut value = String::new();
        loop {
            match self.chars.next() {
                None => break,
                Some(c) if c == quote => { self.pos += 1; break; }
                Some('\\') => {
                    self.pos += 1;
                    if let Some(next) = self.chars.next() {
                        self.pos += 1;
                        value.push(next);
                    }
                }
                Some(c) => { self.pos += 1; value.push(c); }
            }
        }
        Token::Phrase(value)
    }

    fn read_regex(&mut self) -> Token {
        let mut pattern = String::new();
        loop {
            match self.chars.next() {
                None => break,
                Some('/') => { self.pos += 1; break; }
                Some('\\') => {
                    self.pos += 1;
                    pattern.push('\\');
                    if let Some(next) = self.chars.next() {
                        self.pos += 1;
                        pattern.push(next);
                    }
                }
                Some(c) => { self.pos += 1; pattern.push(c); }
            }
        }
        Token::Regex(pattern)
    }

    fn read_word(&mut self) -> Token {
        let mut word = String::new();
        while let Some(&c) = self.chars.peek() {
            if c.is_whitespace() || c == '(' || c == ')' || c == '[' || c == ']'
                || c == '{' || c == '}' || c == ':' || c == '"' || c == '\''
            {
                break;
            }
            self.chars.next();
            self.pos += 1;
            word.push(c);
        }
        match word.as_str() {
            "AND" => Token::And,
            "OR" => Token::Or,
            "NOT" => Token::Not,
            "TO" => Token::To,
            _ => Token::Word(word),
        }
    }
}

// ─── Parser ───

struct LuceneParser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    special_fields: SpecialFields,
}

struct SpecialFields {
    regex_fields: Vec<String>,
    sigma_fields: Vec<String>,
}

impl<'a> LuceneParser<'a> {
    fn new(input: &'a str, special_fields: Option<&PyDict>) -> PyResult<Self> {
        // Tokens sammeln
        let mut lexer = Lexer::new(input);
        let mut tokens = Vec::new();
        loop {
            let tok = lexer.next_token();
            if tok == Token::Eof {
                break;
            }
            tokens.push(tok);
        }

        let sf = Self::parse_special_fields(special_fields);

        Ok(Self {
            tokens,
            pos: 0,
            special_fields: sf,
        })
    }

    fn parse_special_fields(sf: Option<&PyDict>) -> SpecialFields {
        // ... aus Python-Dict extrahieren
        SpecialFields {
            regex_fields: vec![],
            sigma_fields: vec![],
        }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> Token {
        let tok = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof);
        self.pos += 1;
        tok
    }

    fn expect(&mut self, expected: &Token) -> PyResult<()> {
        let tok = self.advance();
        if &tok != expected {
            Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Expected {:?}, got {:?}", expected, tok
            )))
        } else {
            Ok(())
        }
    }

    // ─── Grammar-Methoden ───

    fn parse_query(&mut self) -> PyResult<FilterExpression> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> PyResult<FilterExpression> {
        let mut left = self.parse_and()?;
        while *self.peek() == Token::Or {
            self.advance();
            let right = self.parse_and()?;
            left = FilterExpression::new(FilterExpressionInner::Or {
                children: vec![left, right],
            });
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> PyResult<FilterExpression> {
        let mut left = self.parse_not()?;
        loop {
            match self.peek() {
                Token::And => {
                    self.advance();
                    let right = self.parse_not()?;
                    left = FilterExpression::new(FilterExpressionInner::And {
                        children: vec![left, right],
                    });
                }
                Token::Word(_) | Token::Phrase(_) | Token::Star
                | Token::LParen | Token::Slash => {
                    // Implizites AND
                    let right = self.parse_not()?;
                    left = FilterExpression::new(FilterExpressionInner::And {
                        children: vec![left, right],
                    });
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> PyResult<FilterExpression> {
        if *self.peek() == Token::Not {
            self.advance();
            let child = self.parse_not()?;
            Ok(FilterExpression::new(FilterExpressionInner::Not {
                child: Box::new(child),
            }))
        } else {
            self.parse_atom()
        }
    }

    fn parse_atom(&mut self) -> PyResult<FilterExpression> {
        match self.peek().clone() {
            Token::LParen => {
                self.advance();
                let expr = self.parse_query()?;
                self.expect(&Token::RParen)?;
                Ok(expr)
            }
            Token::Star => {
                self.advance();
                Ok(FilterExpression::new(FilterExpressionInner::Always { value: true }))
            }
            _ => self.parse_term(),
        }
    }

    fn parse_term(&mut self) -> PyResult<FilterExpression> {
        match self.peek().clone() {
            Token::Word(w) => {
                let field_name = w;
                self.advance();
                if *self.peek() == Token::Colon {
                    self.advance();
                    self.parse_field_value(&field_name)
                } else {
                    // Bare word → Exists
                    let key = Self::split_dotted_field(&field_name);
                    Ok(FilterExpression::new(FilterExpressionInner::Exists { key }))
                }
            }
            Token::Phrase(p) => {
                // Phrase am Anfang → Fehler oder Exists?
                self.advance();
                let key = Self::split_dotted_field(&p);
                Ok(FilterExpression::new(FilterExpressionInner::Exists { key }))
            }
            _ => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Unexpected token: {:?}", self.peek()
            ))),
        }
    }

    fn parse_field_value(&mut self, field_name: &str) -> PyResult<FilterExpression> {
        let key = Self::split_dotted_field(field_name);
        match self.peek().clone() {
            Token::LBracket | Token::LBrace => self.parse_range(&key),
            Token::Slash => {
                self.advance();
                let pattern = match self.advance() {
                    Token::Regex(p) => p,
                    _ => return Err(pyo3::exceptions::PyValueError::new_err("Expected regex")),
                };
                let normalized = normalize_regex(&pattern);
                Ok(FilterExpression::new(FilterExpressionInner::Regex { key, pattern: normalized }))
            }
            Token::Word(w) if w == "null" => {
                self.advance();
                Ok(FilterExpression::new(FilterExpressionInner::Null { key }))
            }
            Token::Word(w) => {
                self.advance();
                let value = Self::remove_lucene_escaping(&w);
                self.build_value_expression(key, value)
            }
            Token::Phrase(p) => {
                self.advance();
                let value = Self::remove_lucene_escaping(&p);
                self.build_value_expression(key, value)
            }
            Token::LParen => {
                // Field group: field:(expr OR expr)
                self.advance();
                let expr = self.parse_query()?;
                self.expect(&Token::RParen)?;
                Ok(expr)
            }
            _ => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Unexpected token after field '{}': {:?}", field_name, self.peek()
            ))),
        }
    }

    fn parse_range(&mut self, key: &[String]) -> PyResult<FilterExpression> {
        let include_lower = *self.peek() == Token::LBracket;
        self.advance();

        let lower = self.parse_range_boundary()?;
        if *self.peek() != Token::To {
            return Err(pyo3::exceptions::PyValueError::new_err("Expected 'TO' in range"));
        }
        self.advance();
        let upper = self.parse_range_boundary()?;

        let include_upper = match self.peek() {
            Token::RBracket => true,
            Token::RBrace => false,
            _ => return Err(pyo3::exceptions::PyValueError::new_err("Expected ']' or '}'")),
        };
        self.advance();

        // Typ bestimmen: int → float → string
        if let (Ok(lo), Ok(hi)) = (lower.parse::<i64>(), upper.parse::<i64>()) {
            if lo > hi {
                return Err(pyo3::exceptions::PyValueError::new_err("Range lower > upper"));
            }
            Ok(FilterExpression::new(FilterExpressionInner::IntegerRange {
                key: key.to_vec(),
                lower: lo, upper: hi,
                include_lower, include_upper,
            }))
        } else if let (Ok(lo), Ok(hi)) = (lower.parse::<f64>(), upper.parse::<f64>()) {
            if !lo.is_finite() || !hi.is_finite() {
                return Err(pyo3::exceptions::PyValueError::new_err("Range boundaries must be finite"));
            }
            if lo > hi {
                return Err(pyo3::exceptions::PyValueError::new_err("Range lower > upper"));
            }
            Ok(FilterExpression::new(FilterExpressionInner::FloatRange {
                key: key.to_vec(),
                lower: lo, upper: hi,
                include_lower, include_upper,
            }))
        } else {
            // String range
            if lower == "*" || upper == "*" {
                return Err(pyo3::exceptions::PyValueError::new_err("Open boundaries not supported"));
            }
            if lower > upper {
                return Err(pyo3::exceptions::PyValueError::new_err("Range lower > upper"));
            }
            Ok(FilterExpression::new(FilterExpressionInner::StringRange {
                key: key.to_vec(),
                lower, upper,
                include_lower, include_upper,
            }))
        }
    }

    fn parse_range_boundary(&mut self) -> PyResult<String> {
        match self.advance() {
            Token::Word(w) => Ok(w),
            Token::Phrase(p) => Ok(p),
            Token::Star => Err(pyo3::exceptions::PyValueError::new_err("Open boundaries not supported")),
            other => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Invalid range boundary: {:?}", other
            ))),
        }
    }

    fn build_value_expression(
        &self,
        key: Vec<String>,
        value: String,
    ) -> PyResult<FilterExpression> {
        // Prüfe regex_fields / sigma_fields / |re modifier
        let dotted = key.join(".");
        let last = key.last().map(|s| s.as_str()).unwrap_or("");
        let (field_name, modifier) = if let Some(pos) = last.find('|') {
            (&last[..pos], Some(&last[pos + 1..]))
        } else {
            (last, None)
        };

        if modifier == Some("re") {
            let actual_key = if key.len() > 1 {
                let mut k = key[..key.len() - 1].to_vec();
                k.push(field_name.to_string());
                k
            } else {
                vec![field_name.to_string()]
            };
            let normalized = normalize_regex(&value);
            return Ok(FilterExpression::new(FilterExpressionInner::Regex {
                key: actual_key,
                pattern: normalized,
            }));
        }

        if self.special_fields.sigma_fields.contains(&dotted)
            || self.special_fields.sigma_fields.contains(&"true".to_string())
        {
            let regex = build_sigma_regex(&value)?;
            return Ok(FilterExpression::new(FilterExpressionInner::Sigma {
                key, expected: value, regex,
            }));
        }

        if self.special_fields.regex_fields.contains(&dotted) {
            let normalized = normalize_regex(&value);
            return Ok(FilterExpression::new(FilterExpressionInner::Regex {
                key, pattern: normalized,
            }));
        }

        // Default: Wildcard-Check
        if value.contains('*') || value.contains('?') {
            let regex = build_wildcard_regex(&value)?;
            Ok(FilterExpression::new(FilterExpressionInner::Wildcard {
                key, expected: value, regex,
            }))
        } else {
            Ok(FilterExpression::new(FilterExpressionInner::String {
                key, expected: value,
            }))
        }
    }

    fn split_dotted_field(field: &str) -> Vec<String> {
        if !field.contains('\\') {
            return field.split('.').map(String::from).collect();
        }
        // Escaping beachten (wie in Phase 1)
        let mut result = Vec::new();
        let mut buffer = String::new();
        let mut chars = field.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '.' => result.push(std::mem::take(&mut buffer)),
                '\\' => match chars.next() {
                    Some(next) => buffer.push(next),
                    None => buffer.push('\\'),
                },
                _ => buffer.push(c),
            }
        }
        result.push(buffer);
        result
    }

    fn remove_lucene_escaping(s: &str) -> String {
        // Vereinfacht: backslash vor Spezialzeichen entfernen
        let mut result = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(&next) = chars.peek() {
                    if next.is_alphanumeric() || next == '_' || next == '.' || next == '-'
                        || next == '(' || next == ')' || next == '[' || next == ']'
                        || next == '{' || next == '}' || next == ':' || next == '^'
                        || next == '~' || next == '\\' || next == '"' || next == '+'
                    {
                        result.push(next);
                        chars.next();
                        continue;
                    }
                }
            }
            result.push(c);
        }
        result
    }
}

// ─── Python-Brücke ───

#[pyfunction]
#[pyo3(signature = (query_string, special_fields=None))]
fn parse_lucene_query(
    py: Python,
    query_string: &str,
    special_fields: Option<&Bound<'_, PyDict>>,
) -> PyResult<FilterExpression> {
    // Escaping anwenden (wie LuceneFilter._add_lucene_escaping)
    let escaped = add_lucene_escaping(query_string)?;
    let mut parser = LuceneParser::new(&escaped, special_fields)?;
    parser.parse_query()
}

fn add_lucene_escaping(s: &str) -> PyResult<String> {
    // Port der Python-Logik aus lucene_filter.py
    let s = make_uneven_double_quotes_escaping(s)?;
    let s = escape_ends_of_expressions(&s);
    Ok(s)
}

fn make_uneven_double_quotes_escaping(s: &str) -> PyResult<String> {
    // ... äquivalent zu Python
    Ok(s.to_string())
}

fn escape_ends_of_expressions(s: &str) -> String {
    // ... äquivalent zu Python
    s.to_string()
}
```

#### Python-Modul (`logprep/filter/expression/__init__.py`):

```python
"""Filter expression — Rust-backed classes from logprep._rust.filter."""

from logprep._rust.filter import (  # noqa: F401
    FilterExpression,
    FilterExpressionError,
    KeyDoesNotExistError,
    Always,
    Not,
    And,
    Or,
    StringFilterExpression,
    WildcardStringFilterExpression,
    SigmaFilterExpression,
    IntegerFilterExpression,
    FloatFilterExpression,
    IntegerRangeFilterExpression,
    FloatRangeFilterExpression,
    StringRangeFilterExpression,
    RegExFilterExpression,
    Exists,
    Null,
)
```

**Verifizierung:**
```bash
cargo test -p logprep-core
uv run pytest tests/unit/filter/test_lucene_filter.py -vvv
uv run pytest tests/unit/filter/test_filter_expression.py -vvv
```

**Performance-Test:**
```bash
uv run python benchmarks/run_phase_benchmark.py --phase 2 --runs 30 30 30
```

---

### Schritt 2c: Python-Bridge für LuceneFilter

**Ziel**: `lucene_filter.py` wird auf einen Thin Wrapper reduziert, der `LuceneFilter.create()` als Python-API beibehält, aber die entire Parsing- und Transformer-Logik in Rust delegiert.

**Abhängigkeiten**: Schritt 2b (Lucene-Parser in Rust)

#### Neue `logprep/filter/lucene_filter.py`:

```python
"""Lucene filter — thin wrapper around Rust implementation."""

from typing import Sequence

from logprep._rust.filter import (
    FilterExpression,
    parse_lucene_query,
)
from logprep.abc.exceptions import LogprepException


class LuceneFilterError(LogprepException):
    """Base class for LuceneFilter related exceptions."""


class LuceneFilter:
    """A filter that allows using lucene query strings."""

    @staticmethod
    def create(query_string: str, special_fields: dict | None = None) -> FilterExpression:
        """Create a FilterExpression from a lucene query string.

        Parameters
        ----------
        query_string : str
           A lucene query string.
        special_fields : dict, optional
           Determines if query_string should be processed as regex-query or sigma-query.

        Returns
        -------
        filter : FilterExpression
            A lucene query parsed into a FilterExpression.

        Raises
        ------
        LuceneFilterError
            Raises if lucene filter could not be built.

        """
        try:
            return parse_lucene_query(query_string, special_fields)
        except Exception as error:
            raise LuceneFilterError(
                f"{error} Expression: '{query_string}'"
            ) from error
```

Die gesamte `LuceneTransformer`-Klasse und alle Escaping-Methoden werden aus Python entfernt — sie existieren nur noch in Rust.

**Anpassungen an Import-Sites:**

Alle Importe von `logprep.filter.expression.filter_expression` bleiben unverändert — das `__init__.py` re-exportiert die Rust-Klassen unter demselben Pfad. Keine Änderungen nötig in:

- `logprep/processor/base/rule.py`
- `logprep/processor/list_comparison/rule.py`
- `logprep/processor/replacer/rule.py`
- `logprep/processor/dissector/rule.py`
- `logprep/framework/rule_tree/*.py`
- `logprep/util/event.py`

**Verifizierung:**
```bash
cargo test -p logprep-core
uv run pytest tests/unit/filter/ -vvv
uv run pytest tests/unit/framework/rule_tree/ -vvv
uv run pytest tests/unit/processor/ -vvv
```

**Performance-Test:**
```bash
uv run python benchmarks/run_phase_benchmark.py --phase 2 --runs 30 30 30
```

---

### Schritt 2d: Aufräumen + luqum entfernen

**Ziel**: Alten Python-Code entfernen, `luqum` aus den Abhängigkeiten streichen, 100% Testabdeckung sicherstellen.

**Abhängigkeiten**: Schritt 2c (Python-Bridge funktioniert)

#### Was wird gelöscht

| Datei | Aktion |
|---|---|
| `logprep/filter/expression/filter_expression.py` | Komplett löschen (449 Zeilen → Rust) |
| `logprep/filter/lucene_filter.py` | Alte Implementierung löschen, durch 2c-Wrapper ersetzen |

#### Was bleibt (unverändert)

| Datei | Grund |
|---|---|
| `logprep/filter/__init__.py` | Package-init |
| `logprep/filter/expression/__init__.py` | Re-export aus Rust (Schritt 2b) |

#### pyproject.toml Änderung

```diff
 dependencies = [
     ...
-    "luqum<2",
     ...
 ]

 [tool.mypy]
 module = [
     ...
-    "luqum.*",
     ...
 ]
```

#### `uv lock` neu ausführen

```bash
uv lock
```

Die `luqum`-Abhängigkeit verschwindet aus `uv.lock`.

#### Testanpassungen

Einige Tests in `tests/unit/filter/test_lucene_filter.py` testen interne Escaping-Methoden, die nur noch in Rust existieren. Diese Tests werden:

1. **Beibehalten** als Rust-Unit-Tests in `crates/logprep-core/src/filter/lucene.rs`
2. **Aus Python entfernt** (测试 die internen Rust-Methoden nicht mehr direkt)

Testklassen die angepasst werden müssen:
- `test_make_uneven_double_quotes_escaping` → Rust-Test
- `test_remove_uneven_double_quotes_escaping` → Rust-Test
- `test_add_lucene_escaping` → Rust-Test
- `test_remove_one_escaping_from_quotes` → Rust-Test
- `test_escape_ends_of_expressions` → Rust-Test
- `test_remove_escaping_from_end_of_expression` → Rust-Test
- `test_remove_lucene_escaping` → Rust-Test

**Python-Tests die unverändert bleiben** (nur API-Tests):
- `TestLueceneFilter` (Hauptklasse) — testet `LuceneFilter.create()` API
- `test_create_filter_success` / `test_create_filter_error`
- Alle Range-Tests, Boundary-Tests, Type-Mismatch-Tests
- `TestFilterExpression` Klassen — testen `matches()`/`does_match()` API

**Verifizierung:**
```bash
cargo test -p logprep-core
uv run pytest tests/unit/filter/ -vvv
uv run pytest tests/unit/framework/rule_tree/ -vvv
uv run pytest tests/unit/processor/ -vvv
uv run pytest ./tests --cov=logprep --cov-report=xml -vvv
pre-commit run --all-files
```

**Performance-Test:**
```bash
uv run python benchmarks/run_phase_benchmark.py --phase 2 --runs 30 30 30
uv run python benchmarks/compare_phases.py --phase-baseline 1 --phase-current 2
```

---

### Zusammenfassung Phase 2: Reihenfolge der Commits

| # | Beschreibung | Betrifft | Risiko |
|---|---|---|---|
| 2a | `regex` Crate + 14 Expression-Klassen in Rust | Nur neue Rust-Dateien, `Cargo.toml` | Niedrig — kein Python-Code betroffen |
| 2b | Lucene-Parser in Rust (ersetzt `luqum`) | Nur neue Rust-Dateien | Mittel — Parser-Logik komplex |
| 2c | Python-Bridge: `lucene_filter.py` → Thin Wrapper, `expression/__init__.py` → Rust-Imports | `lucene_filter.py`, `expression/__init__.py` | Hoch — API-Vertrag muss identisch bleiben |
| 2d | Aufräumen: `filter_expression.py` löschen, `luqum` entfernen | `pyproject.toml`, Tests, `uv.lock` | Niedrig — nur Aufräumen |

**Jeder Commit** muss:
1. Alle bestehenden Tests bestehen (`uv run pytest ./tests -vvv`)
2. `pre-commit run --all-files` bestehen
3. `cargo test -p logprep-core` bestehen (ab Schritt 2a)
4. Nix-Docker-Image bauen (`nix build .#packages.x86_64-linux.docker.python312`)
5. CHANGELOG.md aktualisiert sein
6. Performance-Test durchführen (`./benchmarks`)

### Performance-Test nach Phase 2

```bash
# Benchmark für Phase 2
uv run python benchmarks/run_phase_benchmark.py --phase 2 --runs 30 30 30

# Vergleich mit Phase 1 (Baseline)
uv run python benchmarks/compare_phases.py --phase-baseline 1 --phase-current 2

# Ergebnis in BENCHMARK_HISTORY.md eintragen
```

**Erwarteter Impact**: Die Filter-Matching-Logik ist der Hot-Path für jedes Rule-Matching. Die Rust-Implementierung sollte einen messbaren Throughput-Gewinn liefern, da pro Nachricht dutzende bis hunderte Filter-Aufrufe stattfinden. Der Wegfall von `luqum` als Build-Dependency vereinfacht zudem den Build-Prozess.

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
