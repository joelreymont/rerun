#!/usr/bin/env python3
"""
Compare two flamegraph SVG files and generate a performance comparison report.

Usage:
    python compare_flamegraphs.py baseline.svg optimized.svg [--output report.md]
"""

from __future__ import annotations

import re
import argparse
import sys
import xml.etree.ElementTree as ET
from collections import defaultdict
from pathlib import Path
from typing import Dict, Tuple, List, Any, Optional


def parse_flamegraph(svg_file: str) -> Tuple[int, Dict[str, int]]:
    """
    Parse a flamegraph SVG file to extract total samples and function-level samples.

    Returns:
        Tuple of (total_samples, function_samples_dict)

    Raises:
        ValueError: If the file is not a valid flamegraph SVG
    """
    try:
        tree = ET.parse(svg_file)
    except ET.ParseError as e:
        raise ValueError(f"{svg_file} is not valid XML: {e}")

    root = tree.getroot()

    # Check it's an SVG
    if 'svg' not in root.tag.lower():
        raise ValueError(f"{svg_file} does not appear to be an SVG file")

    # SVG namespace handling (some flamegraphs use it, some don't)
    ns = {'svg': 'http://www.w3.org/2000/svg'}

    functions: Dict[str, int] = defaultdict(int)
    max_samples = 0

    # Find all title elements (flamegraphs encode data in titles)
    title_elements = root.findall('.//svg:title', ns)
    if not title_elements:
        # Try without namespace
        title_elements = root.findall('.//title')

    if not title_elements:
        raise ValueError(f"{svg_file} contains no title elements - may not be a flamegraph")

    # Parse title elements to extract function names and sample counts
    for title in title_elements:
        text = title.text
        if not text:
            continue

        # Try multiple common flamegraph formats:
        # Format 1: "func_name (123 samples, 4.5%)"
        match = re.match(r'^(.+?)\s+\((\d+)\s+samples?[,)]', text)
        if match:
            func_name = match.group(1).strip()
            samples = int(match.group(2))
            functions[func_name] += samples
            max_samples = max(max_samples, samples)
            continue

        # Format 2: "func_name 123"
        match = re.match(r'^(.+?)\s+(\d+)$', text)
        if match:
            func_name = match.group(1).strip()
            samples = int(match.group(2))
            functions[func_name] += samples
            max_samples = max(max_samples, samples)
            continue

        # Format 3: Check for total_samples attribute (custom format)
        # This is a fallback for specially formatted flamegraphs

    # Also try to find total_samples attribute (if present)
    with open(svg_file, 'r', encoding='utf-8') as f:
        content = f.read()

    total_match = re.search(r'total_samples="(\d+)"', content)
    if total_match:
        total = int(total_match.group(1))
    else:
        # Calculate total as the maximum sample count (typically the root frame)
        # or sum of all root-level frames
        total = max_samples if max_samples > 0 else sum(functions.values())

    if total == 0 and len(functions) == 0:
        raise ValueError(f"{svg_file} contains no parseable flamegraph data")

    return total, dict(functions)


def group_functions_by_category(functions: Dict[str, int]) -> Dict[str, List[Tuple[str, int]]]:
    """
    Group functions into categories based on their names.
    Returns a dict mapping category names to lists of (function_name, samples) tuples.
    """
    categories: Dict[str, List[Tuple[str, int]]] = defaultdict(list)

    for func, samples in functions.items():
        func_lower = func.lower()

        # More specific matches first
        if 'accounting_allocator' in func_lower or 'note_alloc' in func_lower or 'note_dealloc' in func_lower:
            categories['Memory Accounting'].append((func, samples))
        elif 'query' in func_lower and 'cache' in func_lower:
            categories['Query Cache'].append((func, samples))
        elif 'time_series' in func_lower or 'line_visualizer' in func_lower:
            categories['Time Series Visualization'].append((func, samples))
        elif 'arc' in func_lower and 'drop' in func_lower:
            categories['Reference Counting'].append((func, samples))
        elif 'rayon' in func_lower:
            categories['Rayon Parallel Processing'].append((func, samples))
        elif 'raw_vec' in func_lower or ('vec' in func_lower and 'grow' in func_lower):
            categories['Vector/Allocation Operations'].append((func, samples))
        elif 'alloc' in func_lower or 'dealloc' in func_lower:
            categories['Memory Allocation'].append((func, samples))
        elif 'mutex' in func_lower or 'lock' in func_lower:
            categories['Synchronization'].append((func, samples))
        else:
            categories['Other'].append((func, samples))

    return dict(categories)


def generate_report(
    baseline_file: str,
    optimized_file: str,
    output_file: Optional[str] = None,
    min_samples: int = 100,
    min_change_pct: float = 5.0,
) -> str:
    """
    Generate a comparison report from two flamegraph SVG files.

    Args:
        baseline_file: Path to baseline flamegraph SVG
        optimized_file: Path to optimized flamegraph SVG
        output_file: Optional output file path
        min_samples: Minimum samples to consider a function significant
        min_change_pct: Minimum percentage change to show in top improvements

    Returns:
        Generated markdown report as string
    """
    baseline_name = Path(baseline_file).stem
    optimized_name = Path(optimized_file).stem

    old_total, old_funcs = parse_flamegraph(baseline_file)
    new_total, new_funcs = parse_flamegraph(optimized_file)

    reduction = old_total - new_total
    reduction_pct = 100 * reduction / old_total if old_total > 0 else 0

    # Calculate changes for all functions
    all_funcs = set(old_funcs.keys()) | set(new_funcs.keys())
    func_changes: List[Dict[str, Any]] = []

    for func in all_funcs:
        old_samples = old_funcs.get(func, 0)
        new_samples = new_funcs.get(func, 0)
        change = new_samples - old_samples

        # Better handling of new/removed functions
        if old_samples == 0 and new_samples > 0:
            # New function
            change_pct = 100.0  # Mark as 100% increase
            is_new = True
            is_removed = False
        elif old_samples > 0 and new_samples == 0:
            # Removed function
            change_pct = -100.0
            is_new = False
            is_removed = True
        elif old_samples > 0:
            # Changed function
            change_pct = 100 * change / old_samples
            is_new = False
            is_removed = False
        else:
            # Both zero (shouldn't happen, but handle it)
            change_pct = 0.0
            is_new = False
            is_removed = False

        func_changes.append({
            'name': func,
            'old': old_samples,
            'new': new_samples,
            'change': change,
            'change_pct': change_pct,
            'is_new': is_new,
            'is_removed': is_removed,
            'old_pct': 100 * old_samples / old_total if old_total > 0 else 0,
            'new_pct': 100 * new_samples / new_total if new_total > 0 else 0,
        })

    # Sort by absolute change (biggest changes first)
    func_changes.sort(key=lambda x: abs(x['change']), reverse=True)

    # Find significant improvements (negative changes)
    improvements = [
        f for f in func_changes
        if f['change'] < 0 and (f['old'] >= min_samples or abs(f['change_pct']) >= min_change_pct)
    ]
    improvements.sort(key=lambda x: x['change'])  # Most negative first

    # Find significant regressions (positive changes)
    regressions = [
        f for f in func_changes
        if f['change'] > 0 and (f['new'] >= min_samples or f['change_pct'] >= min_change_pct)
    ]
    regressions.sort(key=lambda x: x['change'], reverse=True)  # Most positive first

    # Group functions by category (combine old and new for complete picture)
    all_functions_for_categorization = {}
    for func, samples in old_funcs.items():
        all_functions_for_categorization[func] = samples
    for func, samples in new_funcs.items():
        if func not in all_functions_for_categorization:
            all_functions_for_categorization[func] = samples
        else:
            # Take max to ensure categorization works
            all_functions_for_categorization[func] = max(
                all_functions_for_categorization[func], samples
            )

    combined_categories = group_functions_by_category(all_functions_for_categorization)

    # Calculate category totals
    category_totals: Dict[str, Dict[str, int]] = defaultdict(lambda: {'old': 0, 'new': 0})

    for func in func_changes:
        # Find which category this function belongs to
        found_category = False
        for cat_name, cat_funcs in combined_categories.items():
            if any(f[0] == func['name'] for f in cat_funcs):
                category_totals[cat_name]['old'] += func['old']
                category_totals[cat_name]['new'] += func['new']
                found_category = True
                break

        if not found_category:
            # Shouldn't happen, but handle it
            category_totals['Other']['old'] += func['old']
            category_totals['Other']['new'] += func['new']

    # Generate markdown report
    lines: List[str] = []
    lines.append("# Flamegraph Performance Comparison")
    lines.append("")
    lines.append(f"**Baseline:** `{baseline_name}`  ")
    lines.append(f"**Optimized:** `{optimized_name}`")
    lines.append("")
    lines.append("## Overall Performance")
    lines.append("")
    lines.append(f"- **Baseline:** {old_total:,} samples")
    lines.append(f"- **Optimized:** {new_total:,} samples")
    lines.append(f"- **Reduction:** {reduction:,} samples")
    lines.append("")

    if reduction > 0:
        lines.append(f"### **✅ {reduction_pct:.1f}% Performance Improvement**")
        lines.append("")
        lines.append(f"This represents approximately **{reduction_pct:.0f}% reduction in CPU time** during profiling, indicating significant performance improvements.")
    elif reduction < 0:
        lines.append(f"### **⚠️ {abs(reduction_pct):.1f}% Performance Regression**")
        lines.append("")
        lines.append(f"The optimized version shows an increase of {abs(reduction):,} samples ({abs(reduction_pct):.1f}%). This indicates a performance regression that should be investigated.")
    else:
        lines.append("### No Overall Change")
        lines.append("")
        lines.append("Total samples are identical between baseline and optimized versions.")

    lines.append("")
    lines.append("---")
    lines.append("")

    # Top improvements section
    if improvements:
        lines.append("## Top Performance Improvements")
        lines.append("")
        lines.append(f"*Showing functions with at least {min_samples:,} samples or {min_change_pct}% change*")
        lines.append("")

        # Show top 10 improvements
        top_improvements = improvements[:10]
        for i, func in enumerate(top_improvements, 1):
            func_short_name = func['name'].split('::')[-1] if '::' in func['name'] else func['name']
            if len(func_short_name) > 80:
                func_short_name = func_short_name[:77] + "..."

            lines.append(f"### {i}. `{func_short_name}`")
            lines.append(f"- **Baseline:** {func['old']:,} samples ({func['old_pct']:.2f}%)")

            if func['is_removed']:
                lines.append(f"- **Optimized:** 0 samples (removed)")
                lines.append(f"- **✅ Reduction: {func['old']:,} samples (100% - function eliminated)**")
            else:
                lines.append(f"- **Optimized:** {func['new']:,} samples ({func['new_pct']:.2f}%)")
                lines.append(f"- **✅ Reduction: {abs(func['change']):,} samples ({abs(func['change_pct']):.1f}%)**")

            lines.append("")

        lines.append("")

    # Category-based analysis
    lines.append("---")
    lines.append("")
    lines.append("## Category Analysis")
    lines.append("")

    # Sort categories by reduction amount
    sorted_categories = sorted(
        category_totals.items(),
        key=lambda x: x[1]['old'] - x[1]['new'],
        reverse=True
    )

    for cat_name, totals in sorted_categories:
        if totals['old'] == 0 and totals['new'] == 0:
            continue

        cat_reduction = totals['old'] - totals['new']
        cat_reduction_pct = 100 * cat_reduction / totals['old'] if totals['old'] > 0 else 0

        lines.append(f"### {cat_name}")
        lines.append(f"- **Baseline:** {totals['old']:,} samples ({100*totals['old']/old_total:.2f}%)")
        lines.append(f"- **Optimized:** {totals['new']:,} samples ({100*totals['new']/new_total:.2f}%)")

        if cat_reduction > 0:
            lines.append(f"- **✅ Reduction: {cat_reduction:,} samples ({cat_reduction_pct:.1f}%)**")
        elif cat_reduction < 0:
            lines.append(f"- **⚠️ Increase: {abs(cat_reduction):,} samples (+{abs(cat_reduction_pct):.1f}%)**")
        else:
            lines.append(f"- **No change**")

        lines.append("")

    # Regressions section (if any significant ones)
    if regressions:
        lines.append("---")
        lines.append("")
        lines.append("## Performance Regressions")
        lines.append("")
        lines.append("The following functions show increased sample counts in the optimized version:")
        lines.append("")

        for i, func in enumerate(regressions[:5], 1):
            func_short_name = func['name'].split('::')[-1] if '::' in func['name'] else func['name']
            if len(func_short_name) > 60:
                func_short_name = func_short_name[:57] + "..."

            if func['is_new']:
                lines.append(f"{i}. **`{func_short_name}`** (NEW)")
                lines.append(f"   - Not present in baseline")
                lines.append(f"   - Optimized: {func['new']:,} samples ({func['new_pct']:.2f}%)")
            else:
                lines.append(f"{i}. **`{func_short_name}`**")
                lines.append(f"   - Baseline: {func['old']:,} samples ({func['old_pct']:.2f}%)")
                lines.append(f"   - Optimized: {func['new']:,} samples ({func['new_pct']:.2f}%)")
                lines.append(f"   - ⚠️ Increase: +{func['change']:,} samples (+{func['change_pct']:.1f}%)")
            lines.append("")

        lines.append("*Note: Increases may indicate different code paths or additional work in the optimized version.*")
        lines.append("")

    # Summary section
    lines.append("---")
    lines.append("")
    lines.append("## Summary")
    lines.append("")

    if reduction > 0:
        lines.append(f"✅ **Overall Result: {reduction_pct:.1f}% Performance Improvement**")
        lines.append("")
        lines.append("The optimization shows measurable improvements:")
        lines.append("")

        # List top 5 improvements by absolute sample reduction
        for i, func in enumerate(improvements[:5], 1):
            func_name = func['name'].split('::')[-1] if '::' in func['name'] else func['name']
            lines.append(f"{i}. **{func_name}**: {abs(func['change']):,} samples saved ({abs(func['change_pct']):.1f}% reduction)")

        if len(improvements) > 5:
            total_other_improvements = sum(abs(f['change']) for f in improvements[5:])
            lines.append(f"... and {len(improvements) - 5} more improvements totaling {total_other_improvements:,} samples")
    elif reduction < 0:
        lines.append(f"⚠️ **Overall Result: {abs(reduction_pct):.1f}% Performance Regression**")
        lines.append("")
        lines.append("The changes resulted in worse performance. Top contributors:")
        lines.append("")
        for i, func in enumerate(regressions[:5], 1):
            func_name = func['name'].split('::')[-1] if '::' in func['name'] else func['name']
            lines.append(f"{i}. **{func_name}**: +{func['change']:,} samples (+{func['change_pct']:.1f}%)")
    else:
        lines.append("**Overall Result: No Performance Change**")
        lines.append("")
        lines.append("The baseline and optimized versions show similar performance profiles.")

    lines.append("")

    report = "\n".join(lines)

    # Write to file if specified
    if output_file:
        output_path = Path(output_file)
        output_path.write_text(report, encoding='utf-8')
        print(f"✅ Report written to: {output_file}")

    return report


def main() -> int:
    """Main entry point."""
    parser = argparse.ArgumentParser(
        description='Compare two flamegraph SVG files and generate a performance comparison report.',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  python compare_flamegraphs.py baseline.svg optimized.svg
  python compare_flamegraphs.py old.svg new.svg --output comparison.md
  python compare_flamegraphs.py old.svg new.svg --min-samples 50 --min-change 10
        """
    )
    parser.add_argument('baseline', help='Path to baseline flamegraph SVG file')
    parser.add_argument('optimized', help='Path to optimized flamegraph SVG file')
    parser.add_argument(
        '--output', '-o',
        help='Output file path for markdown report (default: print to stdout)'
    )
    parser.add_argument(
        '--min-samples',
        type=int,
        default=100,
        help='Minimum samples to consider a function significant (default: 100)'
    )
    parser.add_argument(
        '--min-change',
        type=float,
        default=5.0,
        help='Minimum percentage change to show in detailed analysis (default: 5.0%%)'
    )

    args = parser.parse_args()

    # Validate files exist
    baseline_path = Path(args.baseline)
    optimized_path = Path(args.optimized)

    if not baseline_path.exists():
        print(f"❌ Error: Baseline file not found: {args.baseline}", file=sys.stderr)
        return 1

    if not optimized_path.exists():
        print(f"❌ Error: Optimized file not found: {args.optimized}", file=sys.stderr)
        return 1

    try:
        report = generate_report(
            args.baseline,
            args.optimized,
            args.output,
            min_samples=args.min_samples,
            min_change_pct=args.min_change
        )

        # Print to stdout if no output file specified
        if not args.output:
            print(report)

        return 0

    except ValueError as e:
        print(f"❌ Error: {e}", file=sys.stderr)
        return 1
    except Exception as e:
        print(f"❌ Unexpected error: {e}", file=sys.stderr)
        import traceback
        traceback.print_exc()
        return 1


if __name__ == '__main__':
    sys.exit(main())
