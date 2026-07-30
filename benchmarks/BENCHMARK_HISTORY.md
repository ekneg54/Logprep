# Logprep-ng Benchmark History

This file tracks throughput benchmarks across migration phases to detect regressions and improvements over time.

## Configuration

- **Mode**: logprep-ng (`--ng 1`)
- **Runs**: 3 × 30 seconds measurement window
- **Events per run**: 400,000
- **Pipeline**: `examples/exampledata/config/benchmark_ng_pipeline.yml`
- **Services**: kafka, opensearch
- **Python**: 3.14.2
- **Date**: 2026-07-28

## Results Summary

| Phase | Commit | Weighted (docs/s) | Median (docs/s) | Average (docs/s) | Min (docs/s) | Max (docs/s) | Std Dev | Total Processed | Δ Weighted |
|-------|--------|-------------------|-----------------|------------------|--------------|--------------|---------|-----------------|------------|
| 0 | `34ae4fce` (v20.0.0) | 3,517.89 | 3,563.55 | 3,517.89 | 3,322.16 | 3,667.95 | 177.36 | 316,611 | — (baseline) |
| 1 | `23e0623d` (HEAD) | 3,344.49 | 3,336.59 | 3,344.49 | 3,334.43 | 3,362.46 | 15.60 | 301,007 | -4.93% |
| 2 | `f69ca1a0` (HEAD) | 3,355.57 | 3,336.72 | 3,355.57 | 3,336.63 | 3,393.36 | 32.73 | 302,002 | -4.61% |

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

## Assessment

**PHASE 1 shows a -4.93% throughput regression** compared to the PHASE 0 baseline.
**PHASE 2 shows a -4.61% throughput regression** compared to the PHASE 0 baseline, but is **+0.33% above Phase 1**.

### Key observations

1. **Higher variance in Phase 0**: Phase 0 had a standard deviation of 177.36 docs/s (range 3,322–3,668), while Phase 1 was much more consistent at 15.60 docs/s (range 3,334–3,362). The Phase 0 first-run outlier (3,668) may have been a measurement artifact (warm-up cache effect).

2. **Consistent but lower throughput in Phase 1**: Phase 1 processed ~100K documents per 30s run consistently, whereas Phase 0 varied between 99K–110K. The tighter clustering in Phase 1 suggests more predictable performance but at a lower ceiling.

3. **Statistical significance**: With only 3 runs per phase and ~5% delta, this result is **not statistically significant**. The difference could be noise from infrastructure variability (Kafka startup timing, OpenSearch indexing lag, container overhead). A proper assessment would require 10+ runs per phase or longer measurement windows.

### Recommendation

- Re-run with **longer windows** (e.g., 60s or 90s per run) and **more runs** (5–10) to reduce noise.
- The current result should be treated as **inconclusive** — neither a confirmed regression nor an improvement.

## How to Add Future Phases

```bash
# Run benchmark for the current phase (e.g., phase 2)
uv run python benchmarks/run_phase_benchmark.py --phase 2 --runs 30 30 30

# Compare against baseline
uv run python benchmarks/compare_phases.py --phase-baseline 0 --phase-current 2

# List all recorded phases
uv run python benchmarks/compare_phases.py --list
```

Then update the tables above with the new row.

## Raw Data

- JSONL history: `benchmarks/results/benchmark_history.jsonl`
- Raw output logs: `benchmarks/results/phase*_ng_*.txt`
