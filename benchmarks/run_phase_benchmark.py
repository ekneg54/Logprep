#!/usr/bin/env python3
# pylint: disable=C0103

"""
Phase benchmark runner for logprep-ng.

Wraps benchmark.py and persists results to benchmark_history.jsonl
for cross-phase comparison.

Usage:
    python benchmarks/run_phase_benchmark.py --phase 3
    python benchmarks/run_phase_benchmark.py --phase 3 --runs 30 30 30
    python benchmarks/run_phase_benchmark.py --phase 4 --runs 60
"""

import argparse
import json
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

RESULTS_DIR = Path(__file__).parent / "results"
HISTORY_FILE = RESULTS_DIR / "benchmark_history.jsonl"
BENCHMARK_SCRIPT = Path(__file__).parent.parent / "benchmark.py"


def run_benchmark(phase: int, runs: list[int]) -> str:
    """Run benchmark.py and return its combined stdout/stderr as string."""
    cmd = [
        sys.executable,
        str(BENCHMARK_SCRIPT),
        "--ng",
        "1",
        "--runs",
        *(str(r) for r in runs),
    ]
    print(f"Running: {' '.join(cmd)}")
    result = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        timeout=sum(runs) * 3 + 600,
    )
    output = result.stdout + "\n" + result.stderr
    if result.returncode != 0:
        print(f"Benchmark exited with code {result.returncode}", file=sys.stderr)
        print(output, file=sys.stderr)
    return output


def parse_throughput_from_output(output: str) -> dict:
    """Extract throughput metrics from benchmark.py output."""
    summary = {}

    weighted_match = re.search(r"throughput \(weighted\):\s*([\d,.]+)\s*docs/s", output)
    if weighted_match:
        summary["weighted"] = float(weighted_match.group(1).replace(",", ""))

    median_match = re.search(r"throughput \(median\):\s*([\d,.]+)\s*docs/s", output)
    if median_match:
        summary["median"] = float(median_match.group(1).replace(",", ""))

    average_match = re.search(r"throughput \(average\):\s*([\d,.]+)\s*docs/s", output)
    if average_match:
        summary["average"] = float(average_match.group(1).replace(",", ""))

    minmax_match = re.search(
        r"throughput \(min/max\):\s*([\d,.]+)\s*/\s*([\d,.]+)\s*docs/s", output
    )
    if minmax_match:
        summary["min"] = float(minmax_match.group(1).replace(",", ""))
        summary["max"] = float(minmax_match.group(2).replace(",", ""))

    stdev_match = re.search(r"throughput \(std dev\):\s*([\d,.]+)\s*docs/s", output)
    if stdev_match:
        summary["stdev"] = float(stdev_match.group(1).replace(",", ""))

    total_processed_match = re.search(r"total processed:\s*([\d_]+)", output)
    if total_processed_match:
        summary["total_processed"] = int(total_processed_match.group(1).replace("_", ""))

    total_runtime_match = re.search(r"total runtime:\s*([\d.]+)\s*s", output)
    if total_runtime_match:
        summary["total_runtime_s"] = float(total_runtime_match.group(1))

    return summary


def parse_run_results(output: str) -> list[dict]:
    """Extract individual run results from benchmark.py output."""
    runs = []
    run_blocks = re.findall(
        r"--- RESULT ---\s*"
        r"run_seconds:\s*([\d_]+)\s*"
        r"events generated:\s*([\d_]+)\s*"
        r"generation time:\s*([\d.]+)\s*s\s*"
        r"measurement window:\s*([\d.]+)\s*s\s*"
        r"processed \(OpenSearch\):\s*([\d_]+)\s*"
        r"throughput:\s*([\d,.]+)\s*docs/s",
        output,
    )
    for block in run_blocks:
        runs.append(
            {
                "run_seconds": int(block[0].replace("_", "")),
                "generated": int(block[1].replace("_", "")),
                "generate_s": float(block[2]),
                "window_s": float(block[3]),
                "processed": int(block[4].replace("_", "")),
                "rate": float(block[5].replace(",", "")),
            }
        )
    return runs


def get_git_commit() -> str:
    """Return short git commit hash."""
    try:
        result = subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"],
            capture_output=True,
            text=True,
            check=True,
        )
        return result.stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return "unknown"


def get_python_version() -> str:
    """Return Python version string."""
    return f"{sys.version_info.major}.{sys.version_info.minor}.{sys.version_info.micro}"


def save_results(phase: int, runs: list[dict], summary: dict, raw_output: str) -> None:
    """Save benchmark results to text file and JSONL history."""
    RESULTS_DIR.mkdir(parents=True, exist_ok=True)

    timestamp = datetime.now(timezone.utc).strftime("%Y%m%d_%H%M%S")
    ts_iso = datetime.now(timezone.utc).isoformat()

    txt_path = RESULTS_DIR / f"phase{phase}_ng_{timestamp}.txt"
    txt_path.write_text(raw_output, encoding="utf-8")
    print(f"Raw output saved to: {txt_path}")

    record = {
        "timestamp": ts_iso,
        "phase": phase,
        "mode": "ng",
        "git_commit": get_git_commit(),
        "python_version": get_python_version(),
        "runs": runs,
        "summary": summary,
    }

    with open(HISTORY_FILE, "a", encoding="utf-8") as f:
        f.write(json.dumps(record) + "\n")
    print(f"JSONL record appended to: {HISTORY_FILE}")


def main() -> None:
    """Run benchmark and persist results."""
    parser = argparse.ArgumentParser(description="Run logprep-ng phase benchmark.")
    parser.add_argument(
        "--phase",
        type=int,
        required=True,
        help="Migration phase number (e.g. 3, 4).",
    )
    parser.add_argument(
        "--runs",
        type=int,
        nargs="+",
        default=[30, 30, 30],
        help="Measurement window durations in seconds (one value per run).",
    )
    args = parser.parse_args()

    raw_output = run_benchmark(args.phase, args.runs)

    summary = parse_throughput_from_output(raw_output)
    run_results = parse_run_results(raw_output)

    if not run_results:
        print("ERROR: Could not parse any run results from benchmark output.", file=sys.stderr)
        print("Output snippet:", raw_output[:2000], file=sys.stderr)
        sys.exit(1)

    save_results(args.phase, run_results, summary, raw_output)

    print(f"\n=== Phase {args.phase} Benchmark Complete ===")
    print(f"Runs: {len(run_results)}")
    print(f"Throughput (weighted): {summary.get('weighted', 'N/A'):,.2f} docs/s")
    print(f"Throughput (median):   {summary.get('median', 'N/A'):,.2f} docs/s")
    print(f"Throughput (average):  {summary.get('average', 'N/A'):,.2f} docs/s")


if __name__ == "__main__":
    main()
