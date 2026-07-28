#!/usr/bin/env python3
# pylint: disable=C0103

"""
Micro-benchmarks for dotted-field helper functions.

Benchmarks each helper individually with realistic data sizes to detect
regressions across migration phases.

Usage:
    uv run python ./benchmarks/benchmark_helpers.py --output benchmarks/phase1h.json
    uv run python ./benchmarks/benchmark_helpers.py --filter get_dotted_field_list --runs 5
    uv run python ./benchmarks/benchmark_helpers.py --baseline benchmarks/baseline_phase1b.json --output benchmarks/phase1c.json
"""

import argparse
import gc
import json
import statistics
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent.parent))

from logprep.util.helper import (  # noqa: E402
    add_fields_to,
    field_list_to_dotted_field,
    get_dotted_field_list,
    get_dotted_field_value,
    get_dotted_field_value_with_missing,
    get_dotted_field_values,
    get_field_value,
    get_field_value_no_slice,
    has_dotted_field,
    join_dotted_fields,
    pop_dotted_field_value,
)


# ---------------------------------------------------------------------------
# Test data
# ---------------------------------------------------------------------------

FLAT_EVENT = {f"field_{i}": f"value_{i}" for i in range(20)}

NESTED_EVENT = {
    "source": {"ip": "10.0.0.1", "port": 443, "bytes": 1500},
    "destination": {"ip": "192.168.1.1", "port": 80, "bytes": 500},
    "event": {"action": "allowed", "category": "firewall", "severity": 3},
    "network": {"protocol": "tcp", "transport": "tcp"},
    "observer": {"name": "fw01", "type": "firewall"},
    "tags": ["production", "internal"],
    "logprep": {"processor": {"type": "enricher"}},
}

DEEP_EVENT = {
    "a": {"b": {"c": {"d": {"e": {"f": {"g": {"h": "deep_value"}}}}}}}
}

LARGE_FLAT_EVENT = {f"key_{i:04d}": i for i in range(500)}

DOTTED_FIELDS = ["source.ip", "destination.port", "event.action", "network.protocol"]
FIELD_PATHS = [
    ["source", "ip"],
    ["destination", "port"],
    ["event", "action"],
    ["network", "protocol"],
]

SIMPLE_FIELDS_LIST = [f"field_{i}" for i in range(10)]


# ---------------------------------------------------------------------------
# Benchmark runner
# ---------------------------------------------------------------------------


def bench(name: str, func, runs: int = 10, warmup: int = 2) -> dict:
    """Run a benchmark function and return timing stats."""
    times = []

    for _ in range(warmup):
        func()

    for _ in range(runs):
        gc.collect()
        gc.disable()
        t0 = time.perf_counter_ns()
        func()
        t1 = time.perf_counter_ns()
        gc.enable()
        times.append(t1 - t0)

    times_ms = [t / 1_000_000 for t in times]
    return {
        "name": name,
        "runs": runs,
        "times_ms": times_ms,
        "median_ms": statistics.median(times_ms),
        "mean_ms": statistics.mean(times_ms),
        "stdev_ms": statistics.stdev(times_ms) if len(times_ms) > 1 else 0.0,
        "min_ms": min(times_ms),
        "max_ms": max(times_ms),
    }


# ---------------------------------------------------------------------------
# Benchmark definitions
# ---------------------------------------------------------------------------

BENCHMARKS = {
    "get_dotted_field_list": lambda: [
        get_dotted_field_list("source.ip") for _ in range(1000)
    ],
    "field_list_to_dotted_field": lambda: [
        field_list_to_dotted_field(["source", "ip"]) for _ in range(1000)
    ],
    "field_list_to_dotted_field_generator": lambda: [
        field_list_to_dotted_field(s) for s in [["a", "b", "c"]] * 1000
    ],
    "join_dotted_fields": lambda: [
        join_dotted_fields(("source", "ip")) for _ in range(1000)
    ],
    "get_dotted_field_value": lambda: [
        get_dotted_field_value(NESTED_EVENT, "source.ip") for _ in range(1000)
    ],
    "get_dotted_field_value_missing": lambda: [
        get_dotted_field_value(NESTED_EVENT, "nonexistent.field") for _ in range(1000)
    ],
    "get_dotted_field_value_with_missing": lambda: [
        get_dotted_field_value_with_missing(NESTED_EVENT, "source.ip") for _ in range(1000)
    ],
    "get_dotted_field_value_deep": lambda: [
        get_dotted_field_value(DEEP_EVENT, "a.b.c.d.e.f.g.h") for _ in range(1000)
    ],
    "get_field_value": lambda: [
        get_field_value(NESTED_EVENT, ["source", "ip"]) for _ in range(1000)
    ],
    "get_field_value_no_slice": lambda: [
        get_field_value_no_slice(NESTED_EVENT, ("source", "ip")) for _ in range(1000)
    ],
    "get_dotted_field_values_flat": lambda: [
        get_dotted_field_values(FLAT_EVENT, SIMPLE_FIELDS_LIST) for _ in range(500)
    ],
    "get_dotted_field_values_nested": lambda: [
        get_dotted_field_values(NESTED_EVENT, DOTTED_FIELDS) for _ in range(500)
    ],
    "has_dotted_field_true": lambda: [
        has_dotted_field(NESTED_EVENT, "source.ip") for _ in range(1000)
    ],
    "has_dotted_field_false": lambda: [
        has_dotted_field(NESTED_EVENT, "nonexistent.field") for _ in range(1000)
    ],
    "pop_dotted_field_value": lambda: [
        pop_dotted_field_value(
            {"a": {"b": {"c": 1}}}, "a.b.c"
        )
        for _ in range(1000)
    ],
    "add_fields_to_single": lambda: [
        add_fields_to({}, {"key": "value"}) for _ in range(1000)
    ],
    "add_fields_to_nested": lambda: [
        add_fields_to({}, {"a": {"b": {"c": "value"}}}) for _ in range(1000)
    ],
    "add_fields_to_overwrite": lambda: (
        lambda d: [add_fields_to(d, {"key": f"v{i}"}, overwrite_target=True) for i in range(1000)]
    )({"key": "initial"}),
    "field_list_to_dotted_field_large": lambda: [
        field_list_to_dotted_field([f"part_{i}" for i in range(10)]) for _ in range(500)
    ],
    "get_dotted_field_values_large_event": lambda: [
        get_dotted_field_values(LARGE_FLAT_EVENT, [f"key_{i:04d}" for i in range(50)])
        for _ in range(100)
    ],
}


# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------


def format_result(result: dict) -> str:
    """Format a single benchmark result."""
    return (
        f"  {result['name']:<45s} "
        f"median={result['median_ms']:>10.3f}ms  "
        f"mean={result['mean_ms']:>10.3f}ms  "
        f"stdev={result['stdev_ms']:>8.3f}ms  "
        f"min={result['min_ms']:>10.3f}ms  "
        f"max={result['max_ms']:>10.3f}ms"
    )


def print_results(results: list[dict]) -> None:
    """Print results in a formatted table."""
    print(f"\n{'='*110}")
    print(f"  {'Benchmark':<45s} {'Median':>12s} {'Mean':>12s} {'Stdev':>10s} {'Min':>12s} {'Max':>12s}")
    print(f"  {'-'*45} {'-'*12} {'-'*12} {'-'*10} {'-'*12} {'-'*12}")
    for r in results:
        print(
            f"  {r['name']:<45s} "
            f"{r['median_ms']:>10.3f}ms  "
            f"{r['mean_ms']:>10.3f}ms  "
            f"{r['stdev_ms']:>8.3f}ms  "
            f"{r['min_ms']:>10.3f}ms  "
            f"{r['max_ms']:>10.3f}ms"
        )
    print(f"{'='*110}\n")


def save_results(results: list[dict], output_path: str) -> None:
    """Save results to JSON."""
    output = {
        "benchmarks": {
            r["name"]: {
                "median_ms": r["median_ms"],
                "mean_ms": r["mean_ms"],
                "stdev_ms": r["stdev_ms"],
                "min_ms": r["min_ms"],
                "max_ms": r["max_ms"],
                "runs": r["runs"],
                "times_ms": r["times_ms"],
            }
            for r in results
        }
    }
    Path(output_path).parent.mkdir(parents=True, exist_ok=True)
    Path(output_path).write_text(json.dumps(output, indent=2), encoding="utf-8")
    print(f"Results saved to: {output_path}")


def compare_with_baseline(current: list[dict], baseline_path: str) -> None:
    """Compare current results against a baseline file."""
    baseline_data = json.loads(Path(baseline_path).read_text(encoding="utf-8"))
    baseline = baseline_data.get("benchmarks", {})

    print(f"\n{'='*95}")
    print(f"  {'Benchmark':<45s} {'Baseline':>12s} {'Current':>12s} {'Delta %':>10s} {'Status':>8s}")
    print(f"  {'-'*45} {'-'*12} {'-'*12} {'-'*10} {'-'*8}")

    regressions = 0
    for r in current:
        name = r["name"]
        if name in baseline:
            b_median = baseline[name]["median_ms"]
            c_median = r["median_ms"]
            if b_median > 0:
                delta_pct = ((c_median - b_median) / b_median) * 100
            else:
                delta_pct = 0.0
            status = "REGRESS" if delta_pct > 5.0 else "OK"
            if status == "REGRESS":
                regressions += 1
            print(
                f"  {name:<45s} "
                f"{b_median:>10.3f}ms  "
                f"{c_median:>10.3f}ms  "
                f"{delta_pct:>+8.1f}%  "
                f"{status:>8s}"
            )
        else:
            print(f"  {name:<45s} {'(no baseline)':>12s}")

    print(f"{'='*95}")
    if regressions:
        print(f"\n  WARNING: {regressions} regression(s) detected (>5% slower than baseline)")
    else:
        print(f"\n  OK: No regressions detected")


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> None:
    """Run helper benchmarks."""
    parser = argparse.ArgumentParser(description="Benchmark dotted-field helper functions.")
    parser.add_argument("--runs", type=int, default=10, help="Number of benchmark iterations.")
    parser.add_argument(
        "--filter",
        type=str,
        default=None,
        help="Comma-separated list of benchmark names to run.",
    )
    parser.add_argument("--baseline", type=str, default=None, help="Baseline JSON file to compare.")
    parser.add_argument("--output", type=str, default=None, help="Output JSON file path.")
    args = parser.parse_args()

    if args.filter:
        names = [n.strip() for n in args.filter.split(",")]
        selected = {k: v for k, v in BENCHMARKS.items() if k in names}
        if not selected:
            print(f"ERROR: No matching benchmarks for filter: {args.filter}", file=sys.stderr)
            print(f"Available: {', '.join(BENCHMARKS.keys())}", file=sys.stderr)
            sys.exit(1)
    else:
        selected = BENCHMARKS

    print(f"Running {len(selected)} benchmark(s) with {args.runs} iterations each...\n")

    results = []
    for name, func in selected.items():
        sys.stdout.write(f"  {name}...")
        sys.stdout.flush()
        result = bench(name, func, runs=args.runs)
        results.append(result)
        sys.stdout.write(f" {result['median_ms']:.3f}ms median\n")

    print_results(results)

    if args.output:
        save_results(results, args.output)

    if args.baseline:
        compare_with_baseline(results, args.baseline)


if __name__ == "__main__":
    main()
