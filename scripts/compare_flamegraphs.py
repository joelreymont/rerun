#!/usr/bin/env python3
"""
Compare two flamegraphs (baseline vs optimized) and generate a performance report.

This script compares two flamegraph files and produces a detailed performance report
showing improvements or regressions between the baseline and optimized versions.

Supported formats:
- SVG format (interactive flamegraph SVG files)
- Collapsed stack format (standard flamegraph format): "func1;func2;func3 100"
- JSON format (for tools like puffin)

Usage:
    python compare_flamegraphs.py baseline.svg optimized.svg [--format svg|collapsed|json] [--output report.txt]
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import xml.etree.ElementTree as ET
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass
class StackSample:
    """Represents a stack trace with its sample count."""

    stack: str
    count: int


@dataclass
class FunctionStats:
    """Statistics for a single function."""

    name: str
    self_time: int  # Time spent in this function only
    total_time: int  # Time including children
    count: int  # Number of samples


class FlameGraphData:
    """Container for flamegraph data and analysis."""

    def __init__(self) -> None:
        self.stacks: list[StackSample] = []
        self.function_stats: dict[str, FunctionStats] = {}
        self.total_samples: int = 0

    def add_stack(self, stack: str, count: int) -> None:
        """Add a stack sample to the flamegraph data."""
        self.stacks.append(StackSample(stack=stack, count=count))
        self.total_samples += count

        # Update function statistics
        functions = stack.split(";")
        for i, func in enumerate(functions):
            if func not in self.function_stats:
                self.function_stats[func] = FunctionStats(
                    name=func, self_time=0, total_time=0, count=0
                )

            # Every function in the stack gets the total time
            self.function_stats[func].total_time += count
            self.function_stats[func].count += 1

            # Only the leaf function gets the self time
            if i == len(functions) - 1:
                self.function_stats[func].self_time += count

    def get_function_total_percentage(self, func_name: str) -> float:
        """Get the percentage of total time spent in a function (including children)."""
        if self.total_samples == 0:
            return 0.0
        if func_name not in self.function_stats:
            return 0.0
        return (self.function_stats[func_name].total_time / self.total_samples) * 100

    def get_function_self_percentage(self, func_name: str) -> float:
        """Get the percentage of time spent in a function itself (excluding children)."""
        if self.total_samples == 0:
            return 0.0
        if func_name not in self.function_stats:
            return 0.0
        return (self.function_stats[func_name].self_time / self.total_samples) * 100


def parse_collapsed_format(file_path: Path) -> FlameGraphData:
    """Parse a flamegraph file in collapsed stack format.

    Format: "func1;func2;func3 100"
    Each line contains a semicolon-separated stack trace followed by a sample count.
    """
    data = FlameGraphData()

    with open(file_path) as f:
        for line_num, line in enumerate(f, 1):
            line = line.strip()
            if not line or line.startswith("#"):
                continue

            try:
                # Split stack and count
                parts = line.rsplit(None, 1)
                if len(parts) != 2:
                    print(
                        f"Warning: Skipping malformed line {line_num}: {line}",
                        file=sys.stderr,
                    )
                    continue

                stack, count_str = parts
                count = int(count_str)
                data.add_stack(stack, count)
            except ValueError as e:
                print(
                    f"Warning: Error parsing line {line_num}: {e}", file=sys.stderr
                )
                continue

    return data


def parse_json_format(file_path: Path) -> FlameGraphData:
    """Parse a flamegraph file in JSON format.

    Expected format (puffin-like):
    {
        "stacks": [
            {"stack": ["func1", "func2", "func3"], "count": 100},
            ...
        ]
    }
    """
    data = FlameGraphData()

    with open(file_path) as f:
        json_data = json.load(f)

    # Handle different JSON structures
    stacks = json_data.get("stacks", [])
    if not stacks and isinstance(json_data, list):
        stacks = json_data

    for entry in stacks:
        if isinstance(entry, dict):
            stack_list = entry.get("stack", [])
            count = entry.get("count", entry.get("samples", 1))

            # Convert stack list to semicolon-separated string
            if isinstance(stack_list, list):
                stack = ";".join(str(s) for s in stack_list)
            else:
                stack = str(stack_list)

            data.add_stack(stack, count)

    return data


def parse_svg_format(file_path: Path) -> FlameGraphData:
    """Parse a flamegraph SVG file.

    SVG flamegraphs contain stack information in <title> elements within <g> elements.
    The title typically contains the function stack and sample counts.
    Example: "func1;func2;func3 (100 samples, 5.50%)" or "func1;func2;func3 100"
    """
    data = FlameGraphData()

    try:
        tree = ET.parse(file_path)
        root = tree.getroot()

        # SVG files use namespaces, handle both with and without
        namespaces = {"svg": "http://www.w3.org/2000/svg"}

        # Find all title elements (they contain stack information)
        # Try with namespace first, then without
        titles = root.findall(".//svg:title", namespaces)
        if not titles:
            titles = root.findall(".//title")

        for title in titles:
            if title.text is None:
                continue

            text = title.text.strip()

            # Skip "all" or other meta entries
            if text.lower() in ["all", "flamegraph"]:
                continue

            # Parse the title text
            # Common formats:
            # 1. "func1;func2;func3 (100 samples, 5.50%)"
            # 2. "func1;func2;func3 100"
            # 3. "func1;func2;func3 (100)"

            # Try to extract stack and count
            stack = None
            count = 0

            # Pattern 1: "stack (N samples, X%)" or "stack (N samples)"
            match = re.match(r"^(.+?)\s+\((\d+(?:,\d+)*)\s+samples", text)
            if match:
                stack = match.group(1).strip()
                count_str = match.group(2).replace(",", "")
                count = int(count_str)
            else:
                # Pattern 2: "stack (N)" or "stack N"
                match = re.match(r"^(.+?)\s+\(?(\d+(?:,\d+)*)\)?$", text)
                if match:
                    stack = match.group(1).strip()
                    count_str = match.group(2).replace(",", "")
                    count = int(count_str)
                else:
                    # Fallback: try to find any numbers at the end
                    match = re.match(r"^(.+?)\s+.*?(\d+(?:,\d+)*).*$", text)
                    if match:
                        stack = match.group(1).strip()
                        count_str = match.group(2).replace(",", "")
                        try:
                            count = int(count_str)
                        except ValueError:
                            continue
                    else:
                        # No count found, skip this entry
                        continue

            if stack and count > 0:
                data.add_stack(stack, count)

    except ET.ParseError as e:
        print(f"Error parsing SVG file: {e}", file=sys.stderr)
        raise ValueError(f"Failed to parse SVG file: {e}")

    if data.total_samples == 0:
        print(
            "Warning: No samples found in SVG file. The file may not be a valid flamegraph.",
            file=sys.stderr,
        )

    return data


def parse_flamegraph(file_path: Path, format_type: str = "auto") -> FlameGraphData:
    """Parse a flamegraph file, auto-detecting format if needed."""
    if format_type == "auto":
        # Auto-detect based on file extension or content
        if file_path.suffix.lower() == ".svg":
            format_type = "svg"
        elif file_path.suffix.lower() == ".json":
            format_type = "json"
        else:
            # Try to detect by peeking at first line
            with open(file_path, encoding="utf-8", errors="ignore") as f:
                first_line = f.readline().strip()
                if first_line.startswith("<?xml") or first_line.startswith("<svg"):
                    format_type = "svg"
                elif first_line.startswith("{") or first_line.startswith("["):
                    format_type = "json"
                else:
                    format_type = "collapsed"

    if format_type == "svg":
        return parse_svg_format(file_path)
    elif format_type == "json":
        return parse_json_format(file_path)
    else:
        return parse_collapsed_format(file_path)


@dataclass
class FunctionComparison:
    """Comparison statistics for a single function."""

    name: str
    baseline_total_pct: float
    optimized_total_pct: float
    baseline_self_pct: float
    optimized_self_pct: float
    total_change_pct: float  # Positive = regression, Negative = improvement
    self_change_pct: float


def compare_flamegraphs(
    baseline: FlameGraphData, optimized: FlameGraphData
) -> list[FunctionComparison]:
    """Compare two flamegraphs and return function-level comparisons."""
    all_functions = set(baseline.function_stats.keys()) | set(
        optimized.function_stats.keys()
    )

    comparisons: list[FunctionComparison] = []

    for func_name in all_functions:
        baseline_total = baseline.get_function_total_percentage(func_name)
        optimized_total = optimized.get_function_total_percentage(func_name)
        baseline_self = baseline.get_function_self_percentage(func_name)
        optimized_self = optimized.get_function_self_percentage(func_name)

        # Calculate percentage change
        # Positive = regression (optimized is slower)
        # Negative = improvement (optimized is faster)
        if baseline_total > 0:
            total_change = ((optimized_total - baseline_total) / baseline_total) * 100
        elif optimized_total > 0:
            total_change = float("inf")  # New function added
        else:
            total_change = 0.0

        if baseline_self > 0:
            self_change = ((optimized_self - baseline_self) / baseline_self) * 100
        elif optimized_self > 0:
            self_change = float("inf")
        else:
            self_change = 0.0

        comparisons.append(
            FunctionComparison(
                name=func_name,
                baseline_total_pct=baseline_total,
                optimized_total_pct=optimized_total,
                baseline_self_pct=baseline_self,
                optimized_self_pct=optimized_self,
                total_change_pct=total_change,
                self_change_pct=self_change,
            )
        )

    return comparisons


def generate_report(
    baseline: FlameGraphData,
    optimized: FlameGraphData,
    comparisons: list[FunctionComparison],
    output_path: Path | None = None,
) -> str:
    """Generate a detailed performance comparison report."""
    lines: list[str] = []

    # Header
    lines.append("=" * 80)
    lines.append("FLAMEGRAPH PERFORMANCE COMPARISON REPORT")
    lines.append("=" * 80)
    lines.append("")

    # Overall statistics
    total_baseline = baseline.total_samples
    total_optimized = optimized.total_samples
    overall_change = (
        ((total_optimized - total_baseline) / total_baseline) * 100
        if total_baseline > 0
        else 0.0
    )

    lines.append("Overall Statistics:")
    lines.append(f"  Baseline total samples:  {total_baseline:,}")
    lines.append(f"  Optimized total samples: {total_optimized:,}")
    lines.append(
        f"  Overall change:          {overall_change:+.2f}% "
        f"({'REGRESSION' if overall_change > 0 else 'IMPROVEMENT' if overall_change < 0 else 'NO CHANGE'})"
    )
    lines.append("")

    # Sort comparisons by absolute total change (most significant changes first)
    comparisons_sorted = sorted(
        comparisons,
        key=lambda c: (
            abs(c.total_change_pct) if c.total_change_pct != float("inf") else 0,
            c.baseline_total_pct + c.optimized_total_pct,
        ),
        reverse=True,
    )

    # Top improvements
    lines.append("-" * 80)
    lines.append("TOP 10 IMPROVEMENTS (functions with reduced time)")
    lines.append("-" * 80)
    improvements = [
        c
        for c in comparisons_sorted
        if c.total_change_pct < 0 and c.total_change_pct != float("-inf")
    ]
    for i, comp in enumerate(improvements[:10], 1):
        lines.append(f"\n{i}. {comp.name}")
        lines.append(
            f"   Total time: {comp.baseline_total_pct:.2f}% -> {comp.optimized_total_pct:.2f}% "
            f"({comp.total_change_pct:+.2f}%)"
        )
        lines.append(
            f"   Self time:  {comp.baseline_self_pct:.2f}% -> {comp.optimized_self_pct:.2f}% "
            f"({comp.self_change_pct:+.2f}%)"
        )

    if not improvements:
        lines.append("\nNo improvements detected.")

    # Top regressions
    lines.append("")
    lines.append("-" * 80)
    lines.append("TOP 10 REGRESSIONS (functions with increased time)")
    lines.append("-" * 80)
    regressions = [
        c
        for c in comparisons_sorted
        if c.total_change_pct > 0 and c.total_change_pct != float("inf")
    ]
    for i, comp in enumerate(regressions[:10], 1):
        lines.append(f"\n{i}. {comp.name}")
        lines.append(
            f"   Total time: {comp.baseline_total_pct:.2f}% -> {comp.optimized_total_pct:.2f}% "
            f"({comp.total_change_pct:+.2f}%)"
        )
        lines.append(
            f"   Self time:  {comp.baseline_self_pct:.2f}% -> {comp.optimized_self_pct:.2f}% "
            f"({comp.self_change_pct:+.2f}%)"
        )

    if not regressions:
        lines.append("\nNo regressions detected.")

    # New functions (only in optimized)
    lines.append("")
    lines.append("-" * 80)
    lines.append("NEW FUNCTIONS (only in optimized version)")
    lines.append("-" * 80)
    new_funcs = [c for c in comparisons if c.baseline_total_pct == 0]
    new_funcs_sorted = sorted(
        new_funcs, key=lambda c: c.optimized_total_pct, reverse=True
    )
    for i, comp in enumerate(new_funcs_sorted[:10], 1):
        lines.append(
            f"\n{i}. {comp.name} - {comp.optimized_total_pct:.2f}% (total), {comp.optimized_self_pct:.2f}% (self)"
        )

    if not new_funcs:
        lines.append("\nNo new functions.")

    # Removed functions (only in baseline)
    lines.append("")
    lines.append("-" * 80)
    lines.append("REMOVED FUNCTIONS (only in baseline version)")
    lines.append("-" * 80)
    removed_funcs = [c for c in comparisons if c.optimized_total_pct == 0]
    removed_funcs_sorted = sorted(
        removed_funcs, key=lambda c: c.baseline_total_pct, reverse=True
    )
    for i, comp in enumerate(removed_funcs_sorted[:10], 1):
        lines.append(
            f"\n{i}. {comp.name} - {comp.baseline_total_pct:.2f}% (total), {comp.baseline_self_pct:.2f}% (self)"
        )

    if not removed_funcs:
        lines.append("\nNo removed functions.")

    # Summary
    lines.append("")
    lines.append("=" * 80)
    lines.append("SUMMARY")
    lines.append("=" * 80)
    lines.append(f"Total functions analyzed: {len(comparisons)}")
    lines.append(f"Functions with improvements: {len(improvements)}")
    lines.append(f"Functions with regressions: {len(regressions)}")
    lines.append(f"New functions: {len(new_funcs)}")
    lines.append(f"Removed functions: {len(removed_funcs)}")
    lines.append("")

    if overall_change < -5:
        lines.append("Overall assessment: SIGNIFICANT IMPROVEMENT")
    elif overall_change < 0:
        lines.append("Overall assessment: MINOR IMPROVEMENT")
    elif overall_change > 5:
        lines.append("Overall assessment: SIGNIFICANT REGRESSION")
    elif overall_change > 0:
        lines.append("Overall assessment: MINOR REGRESSION")
    else:
        lines.append("Overall assessment: NO SIGNIFICANT CHANGE")

    lines.append("=" * 80)

    report = "\n".join(lines)

    # Write to file if specified
    if output_path:
        with open(output_path, "w") as f:
            f.write(report)
        print(f"Report written to: {output_path}")

    return report


def main() -> int:
    """Main entry point for the flamegraph comparison tool."""
    parser = argparse.ArgumentParser(
        description="Compare two flamegraphs and generate a performance report.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  # Compare two SVG flamegraph files
  %(prog)s baseline.svg optimized.svg

  # Compare collapsed format flamegraph files
  %(prog)s baseline.txt optimized.txt

  # Compare and save report to file
  %(prog)s baseline.svg optimized.svg --output report.txt

  # Explicitly specify format
  %(prog)s baseline.json optimized.json --format json

Supported formats:
  - svg:       Interactive flamegraph SVG files (most common)
  - collapsed: Standard flamegraph format (func1;func2;func3 100)
  - json:      JSON format with stack arrays
  - auto:      Auto-detect based on file extension (default)
        """,
    )

    parser.add_argument("baseline", type=Path, help="Path to baseline flamegraph file")
    parser.add_argument(
        "optimized", type=Path, help="Path to optimized flamegraph file"
    )
    parser.add_argument(
        "--format",
        choices=["auto", "svg", "collapsed", "json"],
        default="auto",
        help="Format of the flamegraph files (default: auto)",
    )
    parser.add_argument(
        "--output",
        "-o",
        type=Path,
        help="Output file for the report (default: print to stdout)",
    )

    args = parser.parse_args()

    # Validate input files
    if not args.baseline.exists():
        print(f"Error: Baseline file not found: {args.baseline}", file=sys.stderr)
        return 1

    if not args.optimized.exists():
        print(f"Error: Optimized file not found: {args.optimized}", file=sys.stderr)
        return 1

    try:
        # Parse flamegraphs
        print(f"Parsing baseline: {args.baseline}")
        baseline_data = parse_flamegraph(args.baseline, args.format)
        print(f"  Found {baseline_data.total_samples:,} samples")

        print(f"Parsing optimized: {args.optimized}")
        optimized_data = parse_flamegraph(args.optimized, args.format)
        print(f"  Found {optimized_data.total_samples:,} samples")

        # Compare
        print("Comparing flamegraphs...")
        comparisons = compare_flamegraphs(baseline_data, optimized_data)

        # Generate report
        print("Generating report...")
        report = generate_report(
            baseline_data, optimized_data, comparisons, args.output
        )

        # Print to stdout if no output file specified
        if not args.output:
            print("\n")
            print(report)

        return 0

    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        import traceback

        traceback.print_exc()
        return 1


if __name__ == "__main__":
    sys.exit(main())
