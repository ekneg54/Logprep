# Logprep Agent Instructions

## Quick Reference

```bash
# Install (lockfile must be in sync — CI runs `uv lock --check`)
uv sync --frozen --extra dev
pre-commit install

# Test (all)
uv run pytest ./tests --cov=logprep --cov-report=xml -vvv

# Test (single file)
uv run pytest tests/unit/processor/dropper/test_dropper.py -vvv

# Test (single test function)
uv run pytest tests/unit/processor/dropper/test_dropper.py::TestDropper::test_something -vvv

# Lint + format (pre-commit, runs all hooks)
pre-commit run --all-files

# Individual checks (what CI runs)
uv run black --check --diff --config ./pyproject.toml .
uv run pylint <changed-files>
uv run mypy <changed-files>

# Build docs locally
sudo apt install pandoc && uv sync --frozen --extra doc && cd doc && make html

# Rebuild the Rust extension (RELEASE profile is required for benchmarks —
# an unoptimized dev build ~halves ng throughput)
uv run maturin develop --release --manifest-path crates/logprep-core/Cargo.toml

# Run the ng phase benchmark (starts its own Kafka + OpenSearch compose stack)
uv run python benchmarks/run_phase_benchmark.py --phase 3 --runs 30 30 30
```

## Architecture

Logprep is a log processing pipeline: **Input Connector → Processor Chain → Output Connector**.

### Two codebases in parallel

- `logprep/` — Legacy (synchronous multiprocessing). Entry: `logprep.run_logprep:cli`
- `logprep/ng/` — Next-gen (async, uvloop). Entry: `logprep.run_ng:cli`

Both have their own ABCs (`logprep/abc/` and `logprep/ng/abc/`), connectors, processors, and runners. The registry (`logprep/registry.py`) maps component type names to either legacy or ng implementations; `Registry.set_ng_active(True)` switches the active mapping. Tests preload both mappings at session start.

### ng processor orchestration in Rust

Since phase 3.5, event-processing orchestration for ng lives in `PyProcessorCore` (`crates/logprep-core/src/processor/`): rule-tree matching, warning/error handling, the `apply_multiple_times` loop, `delete_source_fields` cleanup, and bypass mode. The ng ABC (`logprep/ng/abc/processor.py`) only consumes the `ProcessOutcome` (`matched_rule_ids`, `warnings`, `errors`) and dispatches un-migrated rules via the `_apply_rule_in_python` callback. `matched_rule_ids` is the sole channel for rule metrics. The processor's `_rule_tree` is a property whose setter binds the live `RuleTree` (via `tree.inner` + `tree.rule_id_to_rule`) to the core; never call `tree.get_matching_rules(...)` from Python.

### Key directories

| Path | Purpose |
|---|---|
| `logprep/abc/` | Abstract base classes (Component, Processor, Connector, Input, Output) |
| `logprep/processor/` | 30+ processors, each in its own subdirectory with `processor.py` + `rule.py` |
| `logprep/ng/` | Next-gen async implementations (abc, connector, processor, event, runner, metrics, util) |
| `logprep/connector/` | Legacy connectors (kafka, opensearch, s3, http, file, json/jsonl) |
| `logprep/ng/connector/` | Ng connectors (fewer implemented) |
| `logprep/generator/` | Event generator connectors (confluent kafka, http) |
| `logprep/framework/` | Pipeline manager and pipeline orchestration |
| `logprep/filter/` | Rule filter engine |
| `logprep/util/` | Helpers, config parsing, time utilities |
| `logprep/metrics/` | Prometheus metrics |
| `logprep/registry.py` | Component type → class path mapping |
| `tests/unit/` | Mirrors `logprep/` structure |
| `tests/acceptance/` | End-to-end tests (require docker compose with Kafka + OpenSearch) |
| `tests/testdata/` | Test fixtures |

### Component pattern

Every processor/connector follows attrs-based configuration:
```python
class MyProcessor(Processor):
    @define(kw_only=True)
    class Config(Processor.Config):
        # params with validators

    @define(kw_only=True)
    class Metrics(Processor.Metrics):
        # CounterMetric / HistogramMetric
```

Each processor implements `_apply_rules()`. Each processor has a corresponding `rule.py` defining filter + action logic.

### Adding a new processor

1. Create `logprep/processor/my_proc/` with `processor.py` and `rule.py`
2. Create `logprep/ng/processor/my_proc/` with ng equivalents
3. Register in `logprep/registry.py` (both `_non_ng_mapping` and `_ng_mapping`)
4. Mirror the structure in `tests/unit/processor/my_proc/`
5. Inherit from `logprep.abc.processor.Processor`

## Code Style

- **Formatter**: Black, line length 100
- **Import sorting**: isort (profile "black")
- **Linter**: pylint, fail-under 9.5, config in `pyproject.toml`
- **Type checking**: mypy, excludes `tests/`
- **YAML formatting**: yamlfmt (via pre-commit)
- **Docstrings**: NumPy style, PEP-257. No docstrings required on tests.
- **Data classes**: Use `attrs` with `@define(kw_only=True)`
- **Type hints**: Required on all code
- **Pre-commit hooks**: trailing-whitespace, end-of-file-fixer, no-commit-to-branch (blocks direct commits to `main`), debug-statements, check-merge-conflict, check-added-large-files, check-toml

## Testing

- Framework: pytest with `asyncio_mode = "auto"` (no need for `@pytest.mark.asyncio` decorators)
- Supported Python: 3.11, 3.12, 3.13, 3.14
- Session start preloads the Registry twice (legacy + ng) — see `tests/conftest.py:pytest_sessionstart`
- Tests must clean up: auto-fixture kills dangling child processes, clears Prometheus registry, clears getter cache
- Coverage aim: 100%
- Acceptance tests in `tests/acceptance/` require external services (kafka, opensearch) via docker compose

## CI (GitHub Actions)

- **PR CI** (`.github/workflows/ci.yml`): runs on PR open/sync, triggers:
  - `uv lock --check` (lockfile must match `pyproject.toml`)
  - CHANGELOG protection (only "Upcoming Changes" section may be modified in PRs)
  - Unit + acceptance tests across Python 3.11–3.14
  - Code quality (black, pylint, mypy on changed files only)
  - Docker compose integration test (`check-examples.yml`) with Kafka + OpenSearch
  - Container build
  - Docs build

## Deprecation Convention

When deprecating a function/feature:
- Log warning: `logger.warning("[Deprecation]: ... [Expires with logprep=X.0.0]")`
- Code comment: `# DEPRECATION: <SHORT-Name> <Comment>`
- Update `CHANGELOG.md` with upcoming removal

## Git Workflow

- Branch naming: `dev-<feature>` or `fix-<issue>`
- PRs target `main`; squash-and-merge only
- Never push directly to `main` (enforced by pre-commit hook)
- Update `CHANGELOG.md` for every feature/improvement/bugfix
- Commit subject ≤50 chars, imperative mood, no period; body wraps at 72 chars
