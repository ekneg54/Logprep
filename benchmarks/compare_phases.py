#!/usr/bin/env python3
# pylint: disable=C0103

"""
Compare benchmark results across migration phases.

Reads benchmark_history.jsonl and compares two phase entries.

Usage:
    python benchmarks/compare_phases.py --phase-baseline 2 --phase-current 3
    python benchmarks/compare_phases.py --list
"""

import argparse
import json
import sys
from pathlib import Path

HISTORY_FILE = Path(__file__).parent / "results" / "benchmark_history.jsonl"


def load_history() -> list[dict]:
    """Load all records from benchmark_history.jsonl."""
    if not HISTORY_FILE.exists():
        print(f"ERROR: History file not found: {HISTORY_FILE}", file=sys.stderr)
        sys.exit(1)
    records = []
    with open(HISTORY_FILE, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                records.append(json.loads(line))
    return records


def list_phases(records: list[dict]) -> None:
    """Print summary of all recorded phases."""
    if not records:
        print("No benchmark records found.")
        return

    print(
        f"{'Phase':<8} {'Timestamp':<28} {'Git Commit':<12} {'Weighted':>12} {'Median':>12} {'Average':>12}"
    )
    print("-" * 88)
    for rec in records:
        summary = rec.get("summary", {})
        print(
            f"{rec.get('phase', '?'):<8} "
            f"{rec.get('timestamp', '?'):<28} "
            f"{rec.get('git_commit', '?'):<12} "
            f"{summary.get('weighted', 0):>10,.2f}  "
            f"{summary.get('median', 0):>10,.2f}  "
            f"{summary.get('average', 0):>10,.2f}"
        )


def find_latest_for_phase(records: list[dict], phase: int) -> dict | None:
    """Find the most recent record for a given phase."""
    matches = [r for r in records if r.get("phase") == phase]
    if not matches:
        return None
    return matches[-1]


def compare_phases(baseline: dict, current: dict) -> None:
    """Print comparison between two phase records."""
    b_summary = baseline.get("summary", {})
    c_summary = current.get("summary", {})

    b_weighted = b_summary.get("weighted", 0)
    c_weighted = c_summary.get("weighted", 0)

    if b_weighted > 0:
        delta_pct = ((c_weighted - b_weighted) / b_weighted) * 100
    else:
        delta_pct = 0.0

    regression = c_weighted < b_weighted

    print("\n=== Phase Comparison ===")
    print(f"{'':30s} {'Baseline':>14s} {'Current':>14s} {'Delta':>14s}")
    print("-" * 76)

    for metric in ["weighted", "median", "average", "min", "max"]:
        b_val = b_summary.get(metric, 0)
        c_val = c_summary.get(metric, 0)
        d = c_val - b_val
        print(f"{metric:>30s} {b_val:>12,.2f}  {c_val:>12,.2f}  {d:>+12,.2f}")

    b_stdev = b_summary.get("stdev", 0)
    c_stdev = c_summary.get("stdev", 0)
    print(f"{'stdev':>30s} {b_stdev:>12,.2f}  {c_stdev:>12,.2f}  {c_stdev - b_stdev:>+12,.2f}")

    b_total = b_summary.get("total_processed", 0)
    c_total = c_summary.get("total_processed", 0)
    print(f"{'total_processed':>30s} {b_total:>12,}  {c_total:>12,}  {c_total - b_total:>+12,}")

    print()
    print(f"Baseline phase: {baseline.get('phase')} ({baseline.get('timestamp', '?')})")
    print(f"Current phase:  {current.get('phase')} ({current.get('timestamp', '?')})")
    print()
    print(f"Throughput change: {delta_pct:+.2f}%")
    if regression:
        print("RESULT: REGRESSION DETECTED — current throughput is lower than baseline")
    else:
        print("RESULT: NO REGRESSION — current throughput is equal or better")


def main() -> None:
    """Compare benchmark results across phases."""
    parser = argparse.ArgumentParser(description="Compare logprep benchmark results across phases.")
    parser.add_argument(
        "--list",
        action="store_true",
        help="List all recorded phase results.",
    )
    parser.add_argument(
        "--phase-baseline",
        type=int,
        default=None,
        help="Baseline phase number.",
    )
    parser.add_argument(
        "--phase-current",
        type=int,
        default=None,
        help="Current phase number to compare against baseline.",
    )
    args = parser.parse_args()

    records = load_history()

    if args.list:
        list_phases(records)
        return

    if args.phase_baseline is None or args.phase_current is None:
        parser.error("Both --phase-baseline and --phase-current are required (or use --list).")

    baseline = find_latest_for_phase(records, args.phase_baseline)
    if baseline is None:
        print(f"ERROR: No record found for phase {args.phase_baseline}", file=sys.stderr)
        sys.exit(1)

    current = find_latest_for_phase(records, args.phase_current)
    if current is None:
        print(f"ERROR: No record found for phase {args.phase_current}", file=sys.stderr)
        sys.exit(1)

    compare_phases(baseline, current)


if __name__ == "__main__":
    main()
