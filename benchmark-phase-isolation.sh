#!/bin/bash
# Isolate Phase 1 vs Phase 2 contributions

set -euo pipefail

echo "==================================================================="
echo "Isolating Phase 1 and Phase 2 Contributions"
echo "==================================================================="
echo ""

# Helper function to measure build time
measure_build() {
    local name="$1"
    local cmd="$2"
    local touch_file="$3"

    echo "Testing: $name"
    cargo clean -p rerun-cli > /dev/null 2>&1
    eval "$cmd" > /dev/null 2>&1  # Initial build

    # Measure incremental rebuild
    touch "$touch_file"
    start=$(date +%s.%N)
    eval "$cmd" > /dev/null 2>&1
    end=$(date +%s.%N)

    runtime=$(echo "$end - $start" | bc)
    echo "  Time: ${runtime}s"
    echo "$runtime"
}

TOUCH_FILE="crates/top/rerun-cli/src/bin/rerun.rs"

# Test 1: Default dev build (Phase 1 + Phase 2 combined)
echo "1. Default dev build (Phase 1 + Phase 2)"
time_default=$(measure_build "Default" "cargo build --bin rerun" "$TOUCH_FILE")
echo ""

# Test 2: Minimal dev build (Phase 1 + Phase 2 combined)
echo "2. Minimal features dev build (Phase 1 + Phase 2)"
time_minimal=$(measure_build "Minimal" "cargo build --bin rerun --no-default-features --features=minimal" "$TOUCH_FILE")
echo ""

# Test 3: Default RELEASE build (Phase 1 only, Phase 2 disabled)
echo "3. Default release build (Phase 2 disabled)"
time_release=$(measure_build "Release default" "cargo build --release --bin rerun" "$TOUCH_FILE")
echo ""

# Test 4: Minimal RELEASE build (Phase 1 only, Phase 2 disabled)
echo "4. Minimal features release build (Phase 1 only)"
time_minimal_release=$(measure_build "Minimal release" "cargo build --release --bin rerun --no-default-features --features=minimal" "$TOUCH_FILE")
echo ""

echo "==================================================================="
echo "Analysis"
echo "==================================================================="
echo ""

# Calculate improvements
phase1_impact=$(echo "scale=1; (($time_release - $time_minimal_release) / $time_release) * 100" | bc)
phase1_plus_2_impact=$(echo "scale=1; (($time_default - $time_minimal) / $time_default) * 100" | bc)
phase2_impact=$(echo "scale=1; $phase1_plus_2_impact - $phase1_impact" | bc)

echo "Build Times:"
echo "  Default dev (P1+P2):      ${time_default}s"
echo "  Minimal dev (P1+P2):      ${time_minimal}s"
echo "  Default release (no P2):  ${time_release}s"
echo "  Minimal release (P1 only): ${time_minimal_release}s"
echo ""
echo "Improvements:"
echo "  Phase 1 + 2 combined:     ${phase1_plus_2_impact}%"
echo "  Phase 1 alone:            ${phase1_impact}%"
echo "  Phase 2 contribution:     ~${phase2_impact}%"
echo ""
echo "Interpretation:"
echo "  - Phase 1 (minimal features) gives: ${phase1_impact}% improvement"
echo "  - Phase 2 (shader skip + graphics opt) adds: ~${phase2_impact}% more"
echo "  - Combined: ${phase1_plus_2_impact}% total improvement"
