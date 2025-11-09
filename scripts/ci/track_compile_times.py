#!/usr/bin/env python3

"""
Track Rust compilation times for the Rerun project.

This script runs cargo build with --timings flag to generate
compilation time reports and optionally stores them for tracking
over time.

Usage:
    python scripts/ci/track_compile_times.py [--clean] [--profile PROFILE] [--output OUTPUT_DIR]
"""

import argparse
import json
import os
import subprocess
import sys
from datetime import datetime
from pathlib import Path


def run_command(cmd: list[str], cwd: str | None = None) -> tuple[int, str, str]:
    """Run a command and return exit code, stdout, stderr."""
    print(f"Running: {' '.join(cmd)}")
    result = subprocess.run(
        cmd,
        cwd=cwd,
        capture_output=True,
        text=True,
    )
    return result.returncode, result.stdout, result.stderr


def track_compile_times(
    clean: bool = False,
    profile: str = "dev",
    output_dir: str | None = None,
    package: str | None = None,
) -> int:
    """
    Track compilation times for the Rerun project.

    Args:
        clean: Whether to run cargo clean first
        profile: Build profile to use (dev, release, etc.)
        output_dir: Directory to store timing data
        package: Specific package to build (e.g., 'rerun')

    Returns:
        Exit code (0 for success, non-zero for failure)
    """
    repo_root = Path(__file__).parent.parent.parent

    if clean:
        print("Running cargo clean...")
        exit_code, stdout, stderr = run_command(
            ["cargo", "clean"],
            cwd=str(repo_root),
        )
        if exit_code != 0:
            print(f"cargo clean failed: {stderr}")
            return exit_code

    # Build the cargo command
    cmd = ["cargo", "build", "--timings"]

    if profile != "dev":
        cmd.extend(["--profile", profile])

    if package:
        cmd.extend(["-p", package])

    # Run cargo build with timings
    print(f"\n{'='*80}")
    print("Tracking compilation times...")
    print(f"{'='*80}\n")

    start_time = datetime.now()
    exit_code, stdout, stderr = run_command(cmd, cwd=str(repo_root))
    end_time = datetime.now()

    duration = (end_time - start_time).total_seconds()

    if exit_code == 0:
        print(f"\n{'='*80}")
        print(f"Build completed successfully in {duration:.2f} seconds")
        print(f"{'='*80}\n")

        # Cargo --timings generates an HTML report in target/cargo-timings/
        timings_dir = repo_root / "target" / "cargo-timings"
        if timings_dir.exists():
            print(f"\nTiming report generated in: {timings_dir}")
            print("Look for cargo-timing-*.html files\n")

        # Store timing data if output directory specified
        if output_dir:
            output_path = Path(output_dir)
            output_path.mkdir(parents=True, exist_ok=True)

            # Create a JSON record of this build
            record = {
                "timestamp": datetime.now().isoformat(),
                "duration_seconds": duration,
                "profile": profile,
                "package": package or "workspace",
                "clean_build": clean,
                "git_commit": os.getenv("GITHUB_SHA", "unknown"),
                "git_ref": os.getenv("GITHUB_REF", "unknown"),
            }

            # Append to a JSONL file for easy parsing
            metrics_file = output_path / "compile_times.jsonl"
            with open(metrics_file, "a") as f:
                f.write(json.dumps(record) + "\n")

            print(f"Timing data recorded to: {metrics_file}")

            # Also copy the HTML report if it exists
            latest_html = None
            if timings_dir.exists():
                html_files = sorted(timings_dir.glob("cargo-timing-*.html"))
                if html_files:
                    latest_html = html_files[-1]
                    import shutil
                    dest = output_path / f"cargo-timing-{datetime.now().strftime('%Y%m%d-%H%M%S')}.html"
                    shutil.copy(latest_html, dest)
                    print(f"HTML report copied to: {dest}")

    else:
        print(f"\n{'='*80}")
        print(f"Build failed after {duration:.2f} seconds")
        print(f"{'='*80}\n")
        print(f"Error output:\n{stderr}")

    return exit_code


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Track Rust compilation times for Rerun"
    )
    parser.add_argument(
        "--clean",
        action="store_true",
        help="Run cargo clean before building",
    )
    parser.add_argument(
        "--profile",
        default="dev",
        help="Build profile to use (default: dev)",
    )
    parser.add_argument(
        "--output",
        help="Directory to store timing data (optional)",
    )
    parser.add_argument(
        "--package",
        "-p",
        help="Specific package to build (e.g., 'rerun')",
    )

    args = parser.parse_args()

    return track_compile_times(
        clean=args.clean,
        profile=args.profile,
        output_dir=args.output,
        package=args.package,
    )


if __name__ == "__main__":
    sys.exit(main())
