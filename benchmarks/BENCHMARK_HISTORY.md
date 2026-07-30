# Logprep-ng Benchmark History

This file tracks throughput benchmarks across migration phases to detect regressions and improvements over time.

## Configuration

- **Mode**: logprep-ng (`--ng 1`)
- **Runs**: 3 × 30 seconds measurement window
- **Events per run**: 400,000
- **Pipeline**: `examples/exampledata/config/benchmark_ng_pipeline.yml`
- **Services**: kafka, opensearch
- **Python**: 3.14.2
- **Date**: 2026-07-30

## Results Summary

| Phase | Commit | Weighted (docs/s) | Median (docs/s) | Average (docs/s) | Min (docs/s) | Max (docs/s) | Std Dev | Total Processed | Δ Weighted |
|-------|--------|-------------------|-----------------|------------------|--------------|--------------|---------|-----------------|------------|
| 0 | `34ae4fce` (v20.0.0) | 3,517.89 | 3,563.55 | 3,517.89 | 3,322.16 | 3,667.95 | 177.36 | 316,611 | — (baseline) |
| 1 | `23e0623d` (HEAD) | 3,344.49 | 3,336.59 | 3,344.49 | 3,334.43 | 3,362.46 | 15.60 | 301,007 | -4.93% |
| 2 | `f69ca1a0` (HEAD) | 3,355.57 | 3,336.72 | 3,355.57 | 3,336.63 | 3,393.36 | 32.73 | 302,002 | -4.61% |
| 3 | `ac604977` | 2,167.97 | 2,167.80 | 2,167.97 | 2,167.76 | 2,168.36 | 0.34 | 195,118 | -38.37% |
| 3a | `ee5682a0` | 3,462.84 | 3,500.56 | 3,462.84 | 3,385.59 | 3,502.36 | 66.90 | 311,656 | -1.56% |

## Detailed Per-Run Results

### Phase 0 — Baseline (`34ae4fce`, v20.0.0)

| Run | Window (s) | Generated | Processed | Throughput (docs/s) |
|-----|------------|-----------|-----------|---------------------|
| 1 | 30.000 | 400,000 | 110,039 | 3,667.95 |
| 2 | 30.000 | 400,000 | 106,907 | 3,563.55 |
| 3 | 30.000 | 400,000 | 99,665 | 3,322.16 |

### Phase 1 — HEAD (`23e0623d`)

| Run | Window (s) | Generated | Processed | Throughput (docs/s) |
|-----|------------|-----------|-----------|---------------------|
| 1 | 30.001 | 400,000 | 100,876 | 3,362.46 |
| 2 | 30.000 | 400,000 | 100,098 | 3,336.59 |
| 3 | 30.000 | 400,000 | 100,033 | 3,334.43 |

### Phase 2 — Rust filter parser + expression refactor (`f69ca1a0`)

| Run | Window (s) | Generated | Processed | Throughput (docs/s) |
|-----|------------|-----------|-----------|---------------------|
| 1 | 30.000 | 400,000 | 100,099 | 3,336.63 |
| 2 | 30.000 | 400,000 | 101,801 | 3,393.36 |
| 3 | 30.000 | 400,000 | 100,102 | 3,336.72 |

### Phase 3 — Rust rule tree + rule parser (`ac604977`)

| Run | Window (s) | Generated | Processed | Throughput (docs/s) |
|-----|------------|-----------|-----------|---------------------|
| 1 | 30.0 | 400,000 | 65,034 | 2,167.80 |
| 2 | 30.0 | 400,000 | 65,033 | 2,167.76 |
| 3 | 30.0 | 400,000 | 65,051 | 2,168.36 |

### Phase 3a — Fixed `get_json_value` clone + cross-type matching (`ee5682a0`)

| Run | Window (s) | Generated | Processed | Throughput (docs/s) |
|-----|------------|-----------|-----------|---------------------|
| 1 | 30.0 | 400,000 | 105,017 | 3,500.56 |
| 2 | 30.0 | 400,000 | 101,568 | 3,385.59 |
| 3 | 30.0 | 400,000 | 105,071 | 3,502.36 |

## Assessment

**PHASE 1 shows a -4.93% throughput regression** compared to the PHASE 0 baseline.
**PHASE 2 shows a -4.61% throughput regression** compared to the PHASE 0 baseline, but is **+0.33% above Phase 1**.
**PHASE 3 shows a -38.37% throughput regression** — the Rust rule tree introduced a major performance bug in `get_json_value` that cloned the entire value on every `matches()` call, plus lost cross-type matching (string/number/bool).
**PHASE 3a recovers to -1.56%** — fixing the `get_json_value` clone (return `&Value` instead of `Value`) and restoring cross-type matching brings throughput from ~2,168 back to ~3,463 docs/s, within 1.6% of the Phase 0 baseline.

### Key observations

1. **Higher variance in Phase 0**: Phase 0 had a standard deviation of 177.36 docs/s (range 3,322–3,668), while Phase 1 was much more consistent at 15.60 docs/s (range 3,334–3,362). The Phase 0 first-run outlier (3,668) may have been a measurement artifact (warm-up cache effect).

2. **Consistent but lower throughput in Phase 1**: Phase 1 processed ~100K documents per 30s run consistently, whereas Phase 0 varied between 99K–110K. The tighter clustering in Phase 1 suggests more predictable performance but at a lower ceiling.

3. **Statistical significance**: With only 3 runs per phase and ~5% delta, this result is **not statistically significant**. The difference could be noise from infrastructure variability (Kafka startup timing, OpenSearch indexing lag, container overhead). A proper assessment would require 10+ runs per phase or longer measurement windows.

### Recommendation

- Re-run with **longer windows** (e.g., 60s or 90s per run) and **more runs** (5–10) to reduce noise.
- The current result should be treated as **inconclusive** — neither a confirmed regression nor an improvement.

## How to Add Future Phases

```bash
# Run benchmark for phase 3 (requires Docker)
uv run python benchmarks/run_phase_benchmark.py --phase 3 --runs 30 30 30

# Compare against previous phase
uv run python benchmarks/compare_phases.py --phase-baseline 2 --phase-current 3

# List all recorded phases
uv run python benchmarks/compare_phases.py --list
```

Then update the tables above with the new row.

## Raw Data

- JSONL history: `benchmarks/results/benchmark_history.jsonl`
- Raw output logs: `benchmarks/results/phase*_ng_*.txt`
