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

**Ziel**: Jeder einfache, rechenintensive Processor wird — inklusive seiner gesamten Rule-Definition, Rule-Validierung, RuleTree-Anbindung und Rule-Anwendung — **komplett in Rust** implementiert. Der Python-Layer für `logprep/ng/processor/<name>/processor.py` ist ein **dünner API-Adapter ohne Event-Verarbeitung**: er lädt Regel-Dicts, reicht sie an die Rust-Implementierung weiter und ruft deren `process(event)` auf. **Kein einziger Prozessor verarbeitet nach Phase 4 noch Events in Python**. Die `rule_tree` wird ausschließlich **aus Rust heraus** konsumiert; Python-Code ruft an keiner Stelle mehr `tree.get_matching_rules(...)` auf. Sämtliche externen Python-Bibliotheken der neun priorisierten Prozessoren (`pyparsing`, `msgspec`, `base64`, Python-`re`, `attrs`, `dataclasses`, `functools.partial`, `functools.cached_property`, `timeout`-Decorator) werden durch Rust-Crates oder direkt in Rust geschriebenen Code ersetzt. Die in Phase 1 migrierten Helper (`pop_dotted_field_value`, `add_fields_to`, `get_dotted_field_value`, …) werden in Phase 4 wiederverwendet, nicht neu implementiert.

**Begründung**:
- Nach Phase 1–3 ist die gesamte Filter-/Rule-Infrastruktur in Rust (`FilterExpressionInner`, `TreeInner`, `RuleParserInner`). Ein Verbleib der Processor-Logik in Python würde den `serde_json::Value` ↔ `dict`-Round-Trip pro Event und Regel erzwingen — der größte verbliebene Hot-Path-Overhead.
- Indem die Rule-Datenstrukturen (z. B. `DropperRule`, `FieldManagerRule`) in Rust definiert und validiert werden, fällt der `attrs`-Validator-Overhead pro `add_rule` weg, und die Validierungslogik kann zwischen `add_rule` und `process` streng typisiert geteilt werden.
- Externe Bibliotheken wie `pyparsing` (Calculator), `msgspec` (Decoder), `attrs` (alle Rule-Klassen) und `re` (Dissector/Replacer) ziehen jeweils C-Extension-Overhead, globale Locks oder großen Speicherbedarf nach sich. Rust-Äquivalente (`meval`/eigener Pratt-Parser, `serde_json`, `serde::Deserialize`, `regex`) sind bereits als Crates verfügbar oder werden in 4a direkt geschrieben.
- Ein dünner Python-Adapter stellt sicher, dass die bestehende `ng.abc.processor.Processor`-Schnittstelle (mit `setup`, `metrics`, `process(event) -> LogEvent`) erhalten bleibt, ohne dass Logik dupliziert wird.

### Leitprinzipien (verbindlich)

1. **Kein Event wird in Python verarbeitet**: Weder `logprep/ng/processor/<name>/processor.py` noch `logprep/ng/abc/processor.py` iterieren über Event-Felder, rufen `pop_dotted_field_value`/`add_fields_to`/`get_dotted_field_value` zur Rule-Anwendung auf oder werten `for rule in matching_rules` aus. Jede `process(event)`-Methode in der Python-Wrapper-Klasse hat exakt die Form:
   ```python
   async def process(self, event):
       self._event = event
       self._rust.process(event.data)
       return self._event
   ```
   (Metrik-Updates erfolgen entweder in Rust oder als ein einziges `for rule in self._rules: rule.metrics.number_of_processed_events += 1` — beides außerhalb des Event-Verarbeitungs-Pfads.)

2. **rule_tree nur via Rust**: `tree.get_matching_rules(...)` wird ausschließlich innerhalb der Rust-Processor-Implementierung aufgerufen. Python-Code sieht nur das Endergebnis (`event.data` mutiert).

3. **Rule komplett in Rust**: Die für den Processor spezifische Rule-Struktur (Felder, Defaults, Validatoren) wird in Rust als eigenes Struct mit `serde::Deserialize` definiert. `attrs` wird nicht mehr für Rule-Klassen verwendet. Python-`Rule`-Klasse bleibt für File-I/O, Filter-Parsing und Metriken bestehen; sie hält jedoch nur noch eine `rule_id`-Referenz auf die echte Rule in der Rust-Instanz.

4. **Externe Python-Bibliotheken → Rust-Crate oder Rust-Implementierung**: Die Tabelle in [§4.0 Crate-Entscheidungen](#40-crate-entscheidungen) ist verbindlich. Keine Phase-4-Implementierung darf eine dort nicht aufgeführte externe Python-Bibliothek einführen.

5. **Phase-1-Helper werden wiederverwendet**: Die in Phase 1 nach Rust portierten Funktionen (`pop_dotted_field_value`, `pop_field_value`, `add_field_to`, `add_field_to_silent_fail`, `add_and_overwrite_key`, `add_and_not_overwrite_key`, `get_dotted_field_value`, `get_dotted_field_value_with_missing`, `get_dotted_field_values`, `has_dotted_field`, `get_source_fields_dict`, `resolve_template`, `add_and_overwrite`, `append`, `append_as_list`, `get_dotted_field_list`, `field_list_to_dotted_field`, `join_dotted_fields`) werden in Phase 4 über `pub(crate) fn` aus `crates/logprep-core/src/field/value.rs` konsumiert (siehe Subphase 4a). **Diese Funktionen werden in Phase 4 nicht reimplementiert.** Deduplizierung wird durch direkten Funktionsaufruf sichergestellt.

6. **Kein Python-Fallback**: Nach Phase 4 gibt es für `pop_dotted_field_value` etc. weiterhin den PyO3-Wrapper (in `crates/logprep-core/src/field/py.rs`) — dieser ist nun aber ein **dünner Adapter**, der `field::value::pop_dotted_field_value` aufruft, nicht umgekehrt. Wer in `process()` der ng-Processor-Klasse noch `pop_dotted_field_value` direkt aufruft, signalisiert, dass die Migration des Prozessors unvollständig ist.

7. **Filter-Parsing bleibt Python-seitig**: `LuceneFilter.create(...)` aus Phase 2 liefert eine `FilterExpression` (Rust-Klasse) zurück. Die `add_rule`-Pipeline des Rust-Prozessors nimmt diesen Filter (via `FilterExpressionInner::from_py_object`) entgegen, parst ihn über den bereits existierenden `RuleParserInner` in Segmente und fügt sie dem internen `TreeInner` hinzu. Der Python-`RuleLoader` bleibt für File-I/O zuständig.

### Architektur (Ist vs. Soll)

**Ist (vor Phase 4)**:
```
Python: Processor.process(event)
  → ng.abc.processor._process_rule_tree(event, self._rule_tree)   # Python-Wrapper
    → tree.get_matching_rules(event)                              # delegiert an Rust
    → for rule in matching_rules: rule.matches(event)             # Python-Methode
    → self._apply_rules(event, rule)                              # Python-Subclass
      → pop_dotted_field_value / add_fields_to / ...              # Python-Helper → Rust
      → pyparsing / msgspec / re / base64 / attrs                 # externe Libs
```

**Soll (nach Phase 4)**:
```
Python: Processor.process(event)               # 3–5 Zeilen, KEINE Event-Verarbeitung
  → rust_processor.process(event)              # 1 Aufruf, alles in Rust
    ↳ TreeInner::get_matching_rules            # Rust (Phase 3)
    ↳ für jede rule_id: RustRule::apply(...)   # Rust
    ↳ field::value::pop_dotted_field_value     # Rust (Phase 1, wiederverwendet)
    ↳ field::value::add_field_to               # Rust (Phase 1, wiederverwendet)
    ↳ meval / serde_json / regex               # Crates statt Python-Libs
    ↳ Event als serde_json::Value mutiert      # 0 Python-Roundtrips pro Rule
```

### 4.0 Crate-Entscheidungen

| Externe Python-Bibliothek | Verwendet in | Rust-Crate / Rust-Implementierung | Crate-Status |
|---|---|---|---|
| `pyparsing` (BNF-Parser) | `calculator/processor.py`, `calculator/fourFn.py` | **Eigener Pratt-Parser** in `crates/logprep-core/src/processor/calculator/expression.rs` (kein neues Crate, um Dependency-Footprint klein zu halten) | in Rust zu schreiben |
| `msgspec.json.Decoder` | `decoder/decoders.py` (`parse_json`, `parse_docker`) | `serde_json` (bereits als `serde_json.workspace = "1"` in `Cargo.toml`) | ✅ bereits vorhanden |
| `base64`, `binascii` | `decoder/decoders.py` (`parse_base64`) | `base64 = "0.22"` Crate | ✅ hinzufügen |
| Python `re` | `dissector/rule.py`, `replacer/rule.py`, `decoder/decoders.py` (alle Regex-Operationen) | `regex` Crate (bereits als `regex.workspace = "1"` in `Cargo.toml`) | ✅ bereits vorhanden |
| `attrs` (`@define`, `validators`) | Alle `rule.py` der 9 Prozessoren | `serde::Deserialize` mit `#[serde(deny_unknown_fields)]` + pro-Feld-`#[serde(default = "...")]` | natives Rust-Pattern |
| `dataclasses.dataclass` | `replacer/rule.py`, `dissector/rule.py` | Native Rust-Structs | nativ |
| `functools.partial` | `decoder/decoders.py` (`partial(_parse, regexes=...)`) | Rust-Closures (`.map_err(...)`, `move |x| ...`) | nativ |
| `functools.cached_property` | `calculator/processor.py` (BNF als Singleton) | `OnceCell` / `OnceLock` aus `std::sync` | nativ |
| `logprep.util.decorators.timeout` | `calculator/processor.py` | `std::sync::mpsc::channel` + `std::thread::spawn` + `recv_timeout` | nativ |
| `logprep.util.helper` (Python-Wrapper) | Alle `processor.py` | `field::value::*` (pure Rust, wiederverwendet) | bereits in Phase 1 migriert |
| `logprep.processor.calculator.fourFn.BNF` | `calculator/processor.py` | Pratt-Parser in Rust (gleicher Funktionsumfang) | in Rust zu schreiben |

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
├── processor/                # (NEU)
│   ├── mod.rs                # ProcessorCore-Trait, RuleSpec-Trait, Registration
│   ├── rule_spec.rs          # gemeinsame Rule-Validierung, SplitFields
│   ├── dropper.rs
│   ├── deleter.rs
│   ├── field_manager.rs      # inkl. Concatenator
│   ├── string_splitter.rs
│   ├── calculator.rs         # inkl. expression.rs (Pratt-Parser)
│   ├── dissector.rs          # inkl. dissect_parser.rs
│   ├── replacer.rs           # inkl. template.rs
│   └── decoder.rs            # inkl. decoders.rs
```

### Datenmodell (Rust)

Jeder Processor definiert drei Typen:

```rust
// 1) Pure Rust Rule — keine PyO3, keine Python-Objekte
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DropperRule {
    pub drop: Vec<String>,
    #[serde(default = "default_drop_full")]
    pub drop_full: bool,
}

// 2) Pure Rust Processor — nutzt field::value::* aus Phase 1
pub struct DropperProcessor {
    name: String,
    rules: HashMap<u64, DropperRule>,
    rule_tree: TreeInner,                  // Phase 3
}

impl DropperProcessor {
    pub fn new(name: String) -> Self { ... }
    pub fn add_rule(&mut self, rule_id: u64, filter: &FilterExpressionInner,
                    raw: &serde_json::Map<String, Value>) -> Result<(), String> { ... }
    pub fn process(&self, event: &mut Value) -> Result<(), String> {
        for rule_id in self.rule_tree.get_matching_rules(event) {
            let rule = &self.rules[&rule_id];
            for dotted in &rule.drop {
                crate::field::value::pop_dotted_field_value(event, dotted, rule.drop_full);
            }
        }
        Ok(())
    }
}

// 3) PyO3-Adapter — dünner Wrapper ohne Event-Logik
#[pyclass]
pub struct PyDropper {
    inner: DropperProcessor,
}

#[pymethods]
impl PyDropper {
    #[new]
    fn new(name: String) -> Self {
        Self { inner: DropperProcessor::new(name) }
    }

    #[pyo3(signature = (rule_id, filter, rule_data))]
    fn add_rule(&mut self, py: Python, rule_id: u64, filter: &Bound<'_, PyAny>,
                rule_data: &Bound<'_, PyDict>) -> PyResult<()> {
        let inner = FilterExpressionInner::from_py_object(filter)?;
        let raw = pydict_to_map(rule_data)?;
        let rule: DropperRule = serde_json::from_value(Value::Object(raw))
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        self.inner.add_rule(rule_id, &inner, &serde_json::from_value(Value::Object(raw)).unwrap())
            .map_err(pyo3::exceptions::PyValueError::new_err)
    }

    fn process(&self, py: Python, event: &Bound<'_, PyDict>) -> PyResult<()> {
        let mut value = pydict_to_json(event)?;
        self.inner.process(&mut value).map_err(pyo3::exceptions::PyValueError::new_err)?;
        json_to_pydict(py, event, &value)
    }

    fn rule_count(&self) -> usize { self.inner.rules.len() }
}
```

### Python-Adapter (Soll) — exakte Form

```python
# logprep/ng/processor/dropper/processor.py
from logprep._rust.processor import PyDropper as _RustDropper
from logprep.ng.abc.processor import Processor
from logprep.processor.dropper.rule import DropperRule
from logprep.util.rule_loader import RuleLoader


class Dropper(Processor):
    """Drop log events. Logik vollständig in Rust (Phase 4)."""

    rule_class = DropperRule

    def __init__(self, name: str, configuration: "Processor.Config") -> None:
        Processor.__init__(self, name, configuration, _rust_processor_factory=_RustDropper)
        self.load_rules(rules_targets=self.config.rules)

    def load_rules(self, rules_targets) -> None:
        for rule in RuleLoader(rules_targets, self.name).rules:
            self._rust.add_rule(
                rule_id=int(rule.id, 16) if isinstance(rule.id, str) else rule.id,
                filter=rule.filter,
                rule_data=rule._config.asdict(),
            )
            self._rule_id_to_rule[rule.id] = rule

    async def process(self, event):
        self._event = event
        self._rust.process(event.data)
        for rule in self._rule_id_to_rule.values():
            rule.metrics.number_of_processed_events += 1
        return self._event
```

> **Negativkatalog**: Diese Imports/Operationen sind im Python-Adapter **verboten**:
> - `from logprep.util.helper import pop_dotted_field_value, add_fields_to, get_dotted_field_value, …`
> - `from pyparsing import …`
> - `import msgspec` (zu Decoder-Zwecken — Decoder ist in Rust)
> - `import re` (für Rule-Logik — Regex ist in Rust)
> - `import base64` (Decoder ist in Rust)
> - `self._rule_tree.get_matching_rules(...)`
> - `for rule in matching_rules: self._apply_rules(event, rule)`
> - `if/elif`-Logik auf Event-Feldern in `process()`
>
> Die einzigen `for`-Schleifen in Python sind: (a) `for rule in RuleLoader(…).rules` in `load_rules` und (b) `for rule in self._rule_id_to_rule.values()` für Metriken. Beide sind außerhalb des Event-Verarbeitungs-Pfads.

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

- **4b — ProcessorCore-Trait + gemeinsame Typen (Pure Rust Core)**: In `crates/logprep-core/src/processor/mod.rs`:
  ```rust
  pub trait ProcessorCore: Send + Sync {
      fn name(&self) -> &str;
      fn add_rule(&mut self, rule_id: u64, filter: &FilterExpressionInner,
                  raw: &serde_json::Map<String, Value>) -> Result<(), String>;
      fn process(&self, event: &mut Value) -> Result<(), String>;
  }
  pub trait RuleSpec: Sized + serde::de::DeserializeOwned + Clone + Send + Sync + std::fmt::Debug {
      const TYPE_NAME: &'static str;
      fn validate(raw: &serde_json::Map<String, Value>) -> Result<(), String>;
  }
  pub struct RuleSlot<R: RuleSpec> {
      pub spec: R,
      pub filter_segments: Vec<FilterExpressionInner>,
  }
  pub struct RuleRegistry<R: RuleSpec> {
      pub rules: HashMap<u64, RuleSlot<R>>,
      pub tree: TreeInner,
  }
  impl<R: RuleSpec> RuleRegistry<R> {
      pub fn add(&mut self, rule_id: u64, filter: &FilterExpressionInner,
                 raw: &serde_json::Map<String, Value>,
                 priority: &HashMap<String, String>, tags: &HashMap<String, String>) -> Result<(), String>;
      pub fn process(&self, event: &mut Value) -> Result<(), String>;  // iteriert tree.matches + wendet spec.apply auf
  }
  ```
  Gemeinsame Helfer:
  - `SplitFields::split(raw: &Map) -> Vec<Vec<String>>` — splittet Dotted-Field-Strings einmalig in `Vec<String>` beim `add_rule`. Verwendet `field::value::get_dotted_field_list`.
  - `validate_required_keys(raw, &["drop"])` — prüft Pflichtschlüssel (ersetzt `attrs`-Validatoren).
  - `processor::register(m)` exportiert alle PyO3-Klassen unter `logprep._rust.processor.*`.
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
  }
  impl DropperRuleSpec {
      pub fn apply(&self, event: &mut Value) {
          for dotted in &self.drop {
              let key = crate::field::value::get_dotted_field_list(dotted);
              crate::field::value::pop_dotted_field_value(event, &key, self.drop_full);
          }
      }
  }
  // ... PyDropper wie im Datenmodell-Abschnitt ...
  ```
  PyO3-Adapter `PyDropper` mit `add_rule(rule_id, filter, rule_data)` und `process(event_dict)`. 30+ Rust-Unit-Tests inkl. Edge-Cases (escaped Dots, leere Dicts, `drop_full=false`, `drop=[]`). Python `logprep/ng/processor/dropper/processor.py` wird auf den 5-Zeilen-Adapter reduziert. Bestehende Dropper-Tests müssen ohne Änderung grün sein.

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

- **4i — Aufräumen + Legacy-Path-Validierung + Negativkatalog-Check**:
  1. `logprep/processor/<name>/processor.py` und `rule.py` werden zu **Re-Exports** (für nicht-ng-Pfad), die `logprep.ng.processor.<name>.processor` importieren. Damit existiert nur noch **eine** Logik-Quelle (Rust) und der nicht-ng-Pfad ist nur ein dünner Python-Alias.
  2. `logprep.registry.Registry._ng_mapping` zeigt weiterhin auf `logprep.ng.processor.<name>.processor.<Name>` — keine Änderung.
  3. `tests/unit/processor/<name>/test_<name>.py` läuft **ohne Änderung** grün — Test-Suite ist die Wahrheit.
  4. **Negativkatalog-Check** (CI-Skript `scripts/check_phase4_adapter_thinness.py`):
     ```bash
     for f in logprep/ng/processor/*/processor.py; do
       grep -E 'pop_dotted_field_value|add_fields_to|get_dotted_field_value|pyparsing|msgspec|^import re|^import base64|_rule_tree\.get_matching_rules' "$f" \
         && { echo "VIOLATION in $f"; exit 1; }
     done
     ```
     Exit-Code ≠ 0 bricht CI.
  5. `benchmarks/results/phase4_<name>_ng_*.txt` wird mit `benchmarks/run_phase_benchmark.py` aufgenommen; Vergleich gegen `phase3_ng_*.txt` darf keine Regression > 5 % zeigen.
  6. `CHANGELOG.md`: Eintrag in „## Upcoming Changes / ### Improvements" pro Processor („`<name>`: migrate core logic + rule spec to Rust via PyO3, replace `<external_lib>` with `<rust_crate>`").

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

- [ ] Alle 9 priorisierten Prozessoren (dropper, deleter, field_manager, concatenator, string_splitter, calculator, dissector, replacer, decoder) sind in `crates/logprep-core/src/processor/<name>.rs` implementiert.
- [ ] `field::value` (Pure-Rust-Versionen der Phase-1-Helper) ist in `crates/logprep-core/src/field/value.rs` vorhanden; `field::py` ist dünner Wrapper.
- [ ] **Kein** `logprep/ng/processor/<name>/processor.py` importiert `pop_dotted_field_value`, `add_fields_to`, `get_dotted_field_value`, `pyparsing`, `msgspec`, `re`, `base64` oder ruft `_rule_tree.get_matching_rules` auf (verifiziert via `scripts/check_phase4_adapter_thinness.py`).
- [ ] **Keine** `for`-Schleife über `event.data`-Keys, `if`-Verzweigungen auf Event-Feldern, oder Event-mutierende Operationen in `process()` der Python-Adapter.
- [ ] Externe Python-Bibliotheken ersetzt: `pyparsing` → eigener Pratt-Parser, `msgspec` → `serde_json`, Python-`re` → `regex` Crate, `base64` → `base64` Crate, `attrs` → `serde::Deserialize`.
- [ ] Phase-1-Helper werden in jeder Processor-Implementierung über `crate::field::value::*` aufgerufen (kein Reimplementieren).
- [ ] Bestehende Unit-Tests aller 9 Prozessoren laufen ohne Modifikation grün.
- [ ] Rust-Unit-Tests pro Processor: ≥ 20 (dropper, deleter), ≥ 40 (field_manager, calculator, dissector, replacer, decoder), ≥ 50 (concatenator, string_splitter). Plus ≥ 30 für `field::value`.
- [ ] Performance: kein Prozessor zeigt > 5 % Regression ggü. Phase-3-Baseline; mindestens die field_manager-/dropper-Klasse zeigen ≥ 10 % Speedup.
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
| Metriken werden in Python via `rule.metrics.number_of_processed_events += 1` aktualisiert; bei Rust-`process` weiß Rust nichts vom Python-`Rule` | (a) Python-Adapter iteriert nach `rust.process()` einmal über die geladenen Rule-IDs (`O(n_rules)`, vernachlässigbar ggü. Event-Verarbeitung) — Default-Variante. (b) Optional: Rust-Processor sammelt `Vec<u64>` der getroffenen Rule-IDs und Python-Adapter nutzt diese — vermeidet Iteration über alle Rules |

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
