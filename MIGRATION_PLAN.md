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

## Phase 1: PyO3-Setup + Dotted-Field Helper

**Ziel**: Rust-Build-Pipeline etablieren, vier zentrale Helper-Funktionen aus `logprep/util/helper.py` schrittweise nach Rust migrieren.

**Begründung**: `get_dotted_field_value` (53 Dateien), `add_fields_to` (43 Dateien), `pop_dotted_field_value` (7 Dateien) und `has_dotted_field` (3 Dateien) sind die meistgenutzten Helfer. Sie sind rein rechenintensiv, haben keine I/O-Abhängigkeiten und bilden die Grundlage für alle Prozessoren.

### Abhängigkeiten

```
1a (Rust scaffolding)
 └─> 1b (maturin + nix)
      └─> 1c (get_dotted_field_list)
           └─> 1d (get_dotted_field_value + _get_item)
                └─> 1e (has_dotted_field)
                └─> 1f (pop_dotted_field_value)
                     └─> 1g (add_fields_to)
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
            └── lib.rs                # empty, nur Platzhalter
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
# Die pyproject-build-systems overlay muss Rust-Toolchain als buildInput bekommen.
# Dafür brauchen wir einen benutzerdefinierten overlay:
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
nix build .#packages.x86_64-linux.python312  # Nix-Package muss bauen
```

**Container-Build mit Nix:**
```bash
nix build .#packages.x86_64-linux.docker.python312 -o image
docker load < image
```

Der Container muss weiterhin funktionieren, weil maturin den Rust-Code beim `pip install` / `uv install` kompiliert.

---

### Schritt 1c: `get_dotted_field_list` in Rust

**Ziel**: Den Dotted-Field-Parser migrieren — das Fundament für alle folgenden Funktionen.

**Warum zuerst**: `get_dotted_field_list` wird von `get_dotted_field_value`, `get_dotted_field_value_with_missing`, `pop_dotted_field_value` und `_pop_field_value` aufgerufen. Er muss zuerst migriert werden.

**Rust-Implementierung** (`crates/logprep-core/src/field.rs`):

```rust
use pyo3::prelude::*;

/// Splitet einen Dotted-Field-String in seine Komponenten.
/// Unterstitzt Escaping: "dotted\.field" → ["dotted.field"]
/// Performance: Ohne Backslash wird str::split(".") verwendet (schnell).
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
            '.' => {
                result.push(std::mem::take(&mut char_buffer));
            }
            '\\' => {
                match chars.next() {
                    Some(next) => char_buffer.push(next),
                    None => char_buffer.push('\\'),
                }
            }
            _ => char_buffer.push(c),
        }
    }
    result.push(char_buffer);
    result
}
```

**PyO3-Modulstruktur** (`crates/logprep-core/src/lib.rs`):

```rust
use pyo3::prelude::*;

mod field;

#[pymodule]
fn _rust(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(field::get_dotted_field_list, m)?)?;
    Ok(())
}
```

**Python-Seite** (`logprep/_rust/__init__.py`):

```python
"""Rust-backed helper functions for logprep."""
from logprep._rust import get_dotted_field_list  # noqa: F401

__all__ = ["get_dotted_field_list"]
```

**Migration in `logprep/util/helper.py`:**

```python
# Ersetze die Python-Funktion durch den Rust-Import:
try:
    from logprep._rust import get_dotted_field_list as _get_dotted_field_list_rust
    def get_dotted_field_list(dotted_field: str) -> Sequence[str]:
        return _get_dotted_field_list_rust(dotted_field)
except ImportError:
    # Fallback für Entwicklung ohne Rust-Build
    def get_dotted_field_list(dotted_field: str) -> Sequence[str]:
        # ... Original-Python-Implementierung ...
```

**Wichtig**: Der `lru_cache` muss entfernt werden — Rust hat eigene Performance, und der Cache ist bei `maxsize=100000` Memory-leak-anfällig. Die Funktion ist in Rust so schnell, dass Caching keinen messbaren Unterschied macht.

**Rust-Tests** (`crates/logprep-core/src/field.rs`):

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
        assert_eq!(
            get_dotted_field_list(r"dotted\\.field"),
            vec![r"dotted\", "field"]
        );
    }

    #[test]
    fn no_dots() {
        assert_eq!(get_dotted_field_list("simple"), vec!["simple"]);
    }

    #[test]
    fn trailing_backslash() {
        assert_eq!(get_dotted_field_list(r"field\\"), vec![r"field\"]);
    }
}
```

**Verifizierung:**
```bash
cargo test -p logprep-core
uv run pytest tests/unit/util/test_helper.py -vvv
```

**Gesamte Testsuite** (nach jedem Schritt):
```bash
uv run pytest ./tests --cov=logprep --cov-report=xml -vvv
pre-commit run --all-files
```

---

### Schritt 1d: `get_dotted_field_value` + `_get_item` in Rust

**Ziel**: Die Kern-Funktion für Feld-Zugriffe migrieren.

**Rust-Implementierung** (`crates/logprep-core/src/field.rs`):

```rust
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PySlice};

/// Low-Level: Einzelnen Key aus einem Container holen.
/// Unterstützt dict-Key, list-Index und list-Slice.
fn get_item<'py>(
    py: Python<'py>,
    container: &Bound<'py, pyo3::PyAny>,
    key: &str,
) -> PyResult<Bound<'py, pyo3::PyAny>> {
    // Versuche dict-Zugriff
    if let Ok(dict) = container.downcast::<PyDict>() {
        return dict.get_item(key)?.ok_or_else(|| {
            pyo3::exceptions::PyKeyError::new_err(key.to_string())
        });
    }

    // List-Zugriff: Index oder Slice
    if let Ok(list) = container.downcast::<PyList>() {
        if key.contains(':') {
            // Slice-Logik
            let parts: Vec<Option<&str>> = key.split(':').map(|s| if s.is_empty() { None } else { Some(s) }).collect();
            let start = parts.get(0).and_then(|s| s.map(|s| s.parse::<isize>().ok()).flatten());
            let stop = parts.get(1).and_then(|s| s.map(|s| s.parse::<isize>().ok()).flatten());
            let step = parts.get(2).and_then(|s| s.map(|s| s.parse::<isize>().ok()).flatten()).unwrap_or(1);
            let slice = PySlice::new(py, start, stop, Some(step))?;
            return list.get_item(&slice);
        }
        let index: isize = key.parse().map_err(|_| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid index: {}", key))
        })?;
        return list.get_item(index);
    }

    Err(pyo3::exceptions::PyTypeError::new_err(
        "Container is neither dict nor list"
    ))
}

/// Traversiert ein verschachteltes Dict mit einem Dotted-Field.
/// Gibt None zurück, wenn das Feld nicht gefunden wird.
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
```

**Migration in `logprep/util/helper.py`:**

```python
try:
    from logprep._rust import get_dotted_field_value as _get_dotted_field_value_rust
    def get_dotted_field_value(event: dict[str, FieldValue], dotted_field: str) -> FieldValue:
        return _get_dotted_field_value_rust(event, dotted_field)
except ImportError:
    # Fallback: Original-Python-Implementierung
    ...
```

**Wichtig**: Die Rust-Funktion arbeitet mit `PyObject` ( beliebige Python-Objekte ). Das bedeutet:
- Kein Serialisierungs-Overhead — Rust navigiert direkt in den Python-Dict-Objekten
- Die Funktion ist eine drop-in-Ersetzung: gleiche Signatur, gleiche Rückgabewerte
- `None` wird zurückgegeben, wenn das Feld nicht gefunden wird (wie in Python)

**Verifizierung:**
```bash
cargo test -p logprep-core
uv run pytest tests/unit/util/test_helper.py::TestGetDottedFieldValue -vvv
uv run pytest tests/unit/util/test_helper.py -vvv  # alle Helper-Tests
```

---

### Schritt 1e: `has_dotted_field` in Rust

**Ziel**: Existenz-Check migrieren — einfache Wrapper-Funktion.

**Rust-Implementierung**:

```rust
/// Prüft ob ein Dotted-Field im Event existiert.
/// Mit allow_none=True (Standard): None-Werte gelten als vorhanden.
/// Mit allow_none=False: None-Werte gelten als nicht vorhanden.
#[pyfunction]
#[pyo3(signature = (event, dotted_field, allow_none=true))]
fn has_dotted_field(
    py: Python,
    event: &Bound<'_, pyo3::PyAny>,
    dotted_field: &str,
    allow_none: bool,
) -> PyResult<bool> {
    let value = get_dotted_field_value(py, event, dotted_field)?;
    if allow_none {
        Ok(!value.is_none(py))
    } else {
        // None → nicht vorhanden
        // Nicht-None → vorhanden (auch wenn der Wert None ist, aber das Feld existiert)
        // ABER: get_dotted_field_value gibt None bei nicht-Gefunden →
        // Wir brauchen get_dotted_field_value_with_missing für allow_none=False
        // → Rust-Variante: Prüfe ob das Feld existiert, nicht ob der Wert None ist
        let parts = get_dotted_field_list(dotted_field);
        let mut current = event.clone();
        for part in &parts {
            current = match get_item(py, &current, part) {
                Ok(val) => val,
                Err(_) => return Ok(false),
            };
        }
        Ok(!current.is_none(py))
    }
}
```

**Alternative (sauberer)**: Rust `get_dotted_field_value_with_missing` implementieren, das einen speziellen Sentinel zurückgibt (z.B. ein Python-Objekt das `MISSING` ist). Dann kann `has_dotted_field` in Python bleiben und die Rust-Funktion nur als Basis nutzen.

**Empfohlener Ansatz**: `has_dotted_field` bleibt in Python, aber nutzt Rust `get_dotted_field_value` als Basis:

```python
# In logprep/util/helper.py — bleibt Python, aber nutzt Rust-Getter:
def has_dotted_field(event, dotted_field, allow_none=True):
    if allow_none:
        return get_dotted_field_value(event, dotted_field) is not None
    return get_dotted_field_value(event, dotted_field) is not MISSING
```

Das ist sicherer, weil `MISSING` ein Python-Sentinel ist und die Rust-Funktion `None` zurückgibt bei nicht-Gefunden.

**Verifizierung:**
```bash
cargo test -p logprep-core
uv run pytest tests/unit/util/test_helper.py -vvv
```

---

### Schritt 1f: `pop_dotted_field_value` in Rust

**Ziel**: Feld-Entfernung mit optionalem Cleanup leerer Dicts.

**Rust-Implementierung**:

```rust
/// Entfernt ein Dotted-Field aus dem Event und gibt den Wert zurück.
/// Gibt MISSING zurück, wenn das Feld nicht existiert.
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
        let result = pop_with_cleanup(py, event, &parts)?;
        Ok(result)
    } else {
        let result = pop_simple(py, event, &parts)?;
        Ok(result)
    }
}

fn pop_simple<'py>(
    py: Python<'py>,
    event: &Bound<'py, pyo3::PyAny>,
    parts: &[String],
) -> PyResult<Bound<'py, pyo3::PyAny>> {
    if parts.is_empty() {
        return Err(pyo3::exceptions::PyValueError::new_err("Empty field path"));
    }

    // Navigiere zum Parent
    let mut current = event.clone();
    for part in &parts[..parts.len() - 1] {
        current = get_item(py, &current, part)?;
    }

    // Pop vom letzten Key
    let last_key = &parts[parts.len() - 1];
    if let Ok(dict) = current.downcast::<PyDict>() {
        dict.pop(last_key, Some(py.None()))
            .map(|v| v.unwrap_or_else(|| py.None()))
    } else {
        Ok(py.None())
    }
}

fn pop_with_cleanup<'py>(
    py: Python<'py>,
    event: &Bound<'py, pyo3::PyAny>,
    parts: &[String],
) -> PyResult<Bound<'py, pyo3::PyAny>> {
    if parts.is_empty() {
        return Err(pyo3::exceptions::PyValueError::new_err("Empty field path"));
    }

    let mut current = event.clone();
    let mut path: Vec<(String, Bound<'py, pyo3::PyAny>)> = Vec::new();

    // Navigiere zum Parent und merke den Weg
    for part in &parts[..parts.len() - 1] {
        let parent = current.clone();
        current = get_item(py, &current, part)?;
        path.push((part.clone(), parent));
    }

    // Pop vom letzten Key
    let last_key = &parts[parts.len() - 1];
    let value = if let Ok(dict) = current.downcast::<PyDict>() {
        dict.pop(last_key, Some(py.None()))
            .map(|v| v.unwrap_or_else(|| py.None()))?
    } else {
        py.None()
    };

    // Cleanup: Entferne leere Dicts den Weg hoch
    for (key, parent) in path.iter().rev() {
        if let Ok(parent_dict) = parent.downcast::<PyDict>() {
            if let Ok(Some(child)) = parent_dict.get_item(key) {
                if let Ok(child_dict) = child.downcast::<PyDict>() {
                    if child_dict.is_empty() {
                        parent_dict.del_item(key)?;
                    }
                }
            }
        }
    }

    Ok(value.unbind())
}
```

**Wichtig**: Die Rust-Funktion gibt ein `PyObject` zurück. `MISSING` muss als Python-Objekt zurückgegeben werden. Entweder:
1. Importiere `MISSING` aus `logprep.util.helper` in Rust (Zirkular-Abhängigkeit!)
2. Erstelle einen `MISSING`-Singleton in Rust und exportiere ihn nach Python
3. Lass Rust `None` zurückgeben und mappes in Python auf `MISSING`

**Empfehlung**: Option 3 — Rust gibt `None` bei Nicht-Gefunden zurück, Python-Wrapper mapped auf `MISSING`:

```python
try:
    from logprep._rust import pop_dotted_field_value as _pop_rust
    def pop_dotted_field_value(event, dotted_field, drop_empty=True):
        result = _pop_rust(event, dotted_field, drop_empty)
        return MISSING if result is None else result
except ImportError:
    # Fallback
    ...
```

Das funktioniert, weil `pop_dotted_field_value` nie `None` als gültigen Wert zurückgibt (der Caller erwartet entweder den Wert oder `MISSING`).

**Verifizierung:**
```bash
cargo test -p logprep-core
uv run pytest tests/unit/util/test_helper.py::TestPopDottedFieldValue -vvv
```

---

### Schritt 1g: `add_fields_to` in Rust

**Ziel**: Die komplexeste Funktion — Batch-Feld-Hinzufügen mit Merge/Overwrite-Logik.

**Begründung**: `add_fields_to` ist die meistverzweigte Funktion mit:
- `FieldExistsWarning` als Python-Exception
- `merge_with_target` (dict→dict: update, list→list: extend, scalar→list: append, list→scalar: append)
- `overwrite_target` (deep copy + Ersetzen)
- `skip_none` (None-Werte filtern)
- Batch-Logik mit partiellen Fehlern

**Rust-Implementierung** (`crates/logprep-core/src/field.rs`):

```rust
use pyo3::exceptions::PyException;

// Custom Exception für FieldExistsWarning
pyo3::create_exception!(logprep_core, FieldExistsWarning, PyException);

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
    let filtered: Vec<(PyObject, PyObject)> = fields.iter()
        .filter(|(k, v)| {
            if skip_none { !v.is_none(py) } else { true }
        })
        .map(|(k, v)| (k.unbind(), v.unbind()))
        .collect();

    if filtered.len() == 1 {
        let (field, value) = &filtered[0];
        add_single_field(py, event, field, value, rule, merge_with_target, overwrite_target)?;
        return Ok(());
    }

    // Batch: Sammle alle fehlgeschlagenen Targets
    let mut unsuccessful = Vec::new();
    for (field, value) in &filtered {
        if let Err(_e) = add_single_field(py, event, field, value, rule, merge_with_target, overwrite_target) {
            unsuccessful.push(field.clone_ref(py));
        }
    }

    if !unsuccessful.is_empty() {
        // FieldExistsWarning mit allen fehlgeschlagenen Targets
        let warning = FieldExistsWarning::new_err((
            rule.map(|r| r.clone_ref(py)).unwrap_or_else(|| py.None()),
            event.clone_ref(py),
            unsuccessful,
        ));
        return Err(warning);
    }

    Ok(())
}
```

**Empfehlung**: `add_fields_to` bleibt initially in Python und nutzt nur `get_dotted_field_value` / `pop_dotted_field_value` aus Rust. Die volle Migration nach Rust erfolgt in einem separaten Commit, nachdem die Basis-Funktionen stabil sind.

**Fallback-Strategie für Phase 1**:
```python
# add_fields_to bleibt Python — zu komplex für den ersten Durchlauf
# Rust-Funktionen werden nur für die Hot-Paths genutzt:
# - get_dotted_field_value (53 Dateien, Hot Path)
# - has_dotted_field (3 Dateien, nutzt get_dotted_field_value)
# - pop_dotted_field_value (7 Dateien, Hot Path)
# add_fields_to wird in Phase 1b (nach Stabilisierung) migriert
```

**Verifizierung:**
```bash
cargo test -p logprep-core
uv run pytest tests/unit/util/test_helper_add_field.py -vvv
uv run pytest tests/unit/util/test_helper.py -vvv
```

---

### Zusammenfassung: Reihenfolge der Commits

| # | Beschreibung | Betrifft | Risiko |
|---|---|---|---|
| 1a | Rust scaffolding (Cargo.toml, lib.rs) | Nur neue Dateien | Minimal |
| 1b | maturin + nix update | pyproject.toml, flake.nix, uv.lock | Hoch — Build-Backend-Wechsel |
| 1c | `get_dotted_field_list` → Rust | helper.py, _rust/__init__.py | Niedrig — isolierte Funktion |
| 1d | `get_dotted_field_value` + `_get_item` → Rust | helper.py | Niedrig — Kern-Funktion |
| 1e | `has_dotted_field` → Rust | helper.py | Minimal — Wrapper |
| 1f | `pop_dotted_field_value` → Rust | helper.py | Mittel — Cleanup-Logik |
| 1g | `add_fields_to` → Rust | helper.py | Hoch — komplexeste Funktion |

**Jeder Commit** muss:
1. Alle bestehenden Tests bestehen (`uv run pytest ./tests -vvv`)
2. `pre-commit run --all-files` bestehen
3. `cargo test -p logprep-core` bestehen (ab Schritt 1c)
4. Nix-Docker-Image bauen (`nix build .#packages.x86_64-linux.docker.python312`)
5. CHANGELOG.md aktualisiert sein

### Container-Build-Verifikation (nach jedem Commit)

```bash
# Nix baut das Docker-Image mit dem geänderten Code
nix build .#packages.x86_64-linux.docker.python312 -o image
docker load < image

# Prüfe dass der Container startet und den CLI-Entrypoint hat
docker run --rm logprep:py312 logprep --help

# Prüfe dass Rust-Module importiert werden kann
docker run --rm logprep:py312 python -c "from logprep._rust import get_dotted_field_list; print('OK')"
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
