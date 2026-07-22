# Logprep Agent Instructions

## Quick Reference

```bash
# Install
uv sync --frozen --extra dev
pre-commit install

# Test (all)
uv run pytest ./tests --cov=logprep --cov-report=xml -vvv

# Test (single file)
uv run pytest tests/unit/processor/dropper/test_dropper.py -vvv

# Test (single test function)
uv run pytest tests/unit/processor/dropper/test_dropper.py::TestDropper::test_something -vvv

# Lint + format (pre-commit)
pre-commit run --all-files

# Build docs locally
sudo apt install pandoc && uv sync --frozen --extra doc && cd doc && make html
```

## Architecture

Logprep is a log processing pipeline: **Input Connector → Processor Chain → Output Connector**.

### Two codebases in parallel

- `logprep/` — Legacy (synchronous multiprocessing). Entry: `logprep.run_logprep:cli`
- `logprep/ng/` — Next-gen (async, uvloop). Entry: `logprep.run_ng:cli`

Both have their own ABCs, connectors, processors, and runners. The registry (`logprep/registry.py`) maps component type names to either legacy or ng implementations; `Registry.set_ng_active(True)` switches the active mapping.

### Key directories

| Path | Purpose |
|---|---|
| `logprep/abc/` | Abstract base classes (Component, Processor, Connector, Input, Output) |
| `logprep/processor/` | 30+ processors, each in its own subdirectory with `processor.py` + `rule.py` |
| `logprep/connector/` | Legacy connectors (kafka, opensearch, s3, http, file, json/jsonl) |
| `logprep/ng/connector/` | Ng connectors (fewer implemented) |
| `logprep/framework/` | Pipeline manager and pipeline orchestration |
| `logprep/filter/` | Rule filter engine |
| `logprep/util/` | Helpers, config parsing, time utilities |
| `logprep/metrics/` | Prometheus metrics |
| `logprep/registry.py` | Component type → class path mapping |
| `tests/unit/` | Mirrors `logprep/` structure |
| `tests/acceptance/` | End-to-end integration tests |
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
- **Pre-commit hooks**: trailing-whitespace, end-of-file-fixer, no-commit-to-branch, debug-statements, check-merge-conflict, check-added-large-files, check-toml

## Testing

- Framework: pytest with `asyncio_mode = "auto"` (no need for `@pytest.mark.asyncio` decorators)
- Session start preloads the Registry twice (legacy + ng) — see `tests/conftest.py:pytest_sessionstart`
- Tests must clean up: auto-fixture kills dangling child processes, clears Prometheus registry, clears getter cache
- Coverage aim: 100%
- Acceptance tests in `tests/acceptance/` require external services (kafka, opensearch)

## Deprecation Convention

When deprecating a function/feature:
- Log warning: `logger.warning("[Deprecation]: ... [Expires with logprep=X.0.0]")`
- Code comment: `# DEPRECATION: <SHORT-Name> <Comment>`
- Update `CHANGELOG.md` with upcoming removal

## Git Workflow

- Branch naming: `dev-<feature>` or `fix-<issue>`
- PRs target `main`; squash-and-merge only
- Never push directly to `main`
- Update `CHANGELOG.md` for every feature/improvement/bugfix
