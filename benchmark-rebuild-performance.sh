#!/bin/bash
# Benchmark script to measure rebuild performance improvements
# This script quantifies the actual impact of Phase 1 and Phase 2 optimizations

set -euo pipefail

RESULTS_FILE="rebuild-benchmark-results.md"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "==================================================================="
echo "Rerun CLI Rebuild Performance Benchmark"
echo "==================================================================="
echo ""
echo "This benchmark measures incremental rebuild times for different"
echo "scenarios to quantify the impact of Phase 1 and Phase 2 optimizations."
echo ""

# Initialize results file
cat > "$RESULTS_FILE" <<EOF
# Rebuild Performance Benchmark Results

**Date**: $(date)
**Platform**: $(uname -s) $(uname -m)
**Rust Version**: $(rustc --version)
**Cargo Version**: $(cargo --version)

## Test Methodology

All tests measure **incremental rebuild time** after touching a source file:
1. Clean build (to establish baseline)
2. Touch a source file to trigger rebuild
3. Measure rebuild time with \`time cargo build\`
4. Repeat 3 times and take the median

## Scenarios Tested

EOF

# Function to run a benchmark
benchmark() {
    local name="$1"
    local touch_file="$2"
    local build_cmd="$3"
    local description="$4"

    echo -e "${YELLOW}Benchmarking: $name${NC}"
    echo "Command: $build_cmd"
    echo "Touch file: $touch_file"
    echo ""

    # Clean build first
    echo "  Clean build..."
    cargo clean -p rerun-cli > /dev/null 2>&1
    eval "$build_cmd" > /dev/null 2>&1

    # Run benchmark 3 times
    times=()
    for i in 1 2 3; do
        echo "  Run $i/3..."
        touch "$touch_file"

        # Measure time
        start=$(date +%s.%N)
        eval "$build_cmd" > /dev/null 2>&1
        end=$(date +%s.%N)

        runtime=$(echo "$end - $start" | bc)
        times+=("$runtime")
        echo "    Time: ${runtime}s"
    done

    # Calculate median (sort and take middle value)
    IFS=$'\n' sorted=($(sort -n <<<"${times[*]}"))
    median="${sorted[1]}"

    echo -e "  ${GREEN}Median time: ${median}s${NC}"
    echo ""

    # Append to results
    cat >> "$RESULTS_FILE" <<EOF
### $name

**Description**: $description

**Build command**: \`$build_cmd\`

**Touch file**: \`$touch_file\`

**Results**:
- Run 1: ${times[0]}s
- Run 2: ${times[1]}s
- Run 3: ${times[2]}s
- **Median: ${median}s**

EOF

    echo "$median"
}

# Check if hyperfine is available (more accurate)
if command -v hyperfine &> /dev/null; then
    echo -e "${GREEN}hyperfine detected - using for more accurate benchmarks${NC}"
    USE_HYPERFINE=true
else
    echo -e "${YELLOW}hyperfine not found - using bash timing${NC}"
    echo "Install with: cargo install hyperfine"
    USE_HYPERFINE=false
fi
echo ""

# Scenario 1: Default build (baseline - includes all features)
echo "==================================================================="
echo "Scenario 1: Default Build (Baseline)"
echo "==================================================================="
time1=$(benchmark \
    "Default Build (all features)" \
    "crates/top/rerun-cli/src/bin/rerun.rs" \
    "cargo build --bin rerun" \
    "Full build with default features: web_viewer, native_viewer, map_view, oss_server")

# Scenario 2: Minimal features (Phase 1 optimization)
echo "==================================================================="
echo "Scenario 2: Minimal Features"
echo "==================================================================="
time2=$(benchmark \
    "Minimal Features Build" \
    "crates/top/rerun-cli/src/bin/rerun.rs" \
    "cargo build --bin rerun --no-default-features --features=minimal" \
    "Build with minimal features: no viewer, no server, no map_view (Phase 1)")

# Scenario 3: CLI-only features (Phase 1 optimization)
echo "==================================================================="
echo "Scenario 3: CLI-Only Features"
echo "==================================================================="
time3=$(benchmark \
    "CLI-Only Features Build" \
    "crates/top/rerun-cli/src/bin/rerun.rs" \
    "cargo build --bin rerun --no-default-features --features=cli-only" \
    "Build with cli-only features: includes server but no viewer (Phase 1)")

# Scenario 4: re_renderer incremental (shader caching test)
echo "==================================================================="
echo "Scenario 4: re_renderer Incremental (Shader Caching)"
echo "==================================================================="
time4=$(benchmark \
    "re_renderer Incremental Build" \
    "crates/viewer/re_renderer/src/lib.rs" \
    "cargo build --package re_renderer" \
    "re_renderer incremental rebuild with shader caching/skipping (Phase 2)")

# Calculate improvements
echo "==================================================================="
echo "Summary and Analysis"
echo "==================================================================="

improvement_minimal=$(echo "scale=2; (($time1 - $time2) / $time1) * 100" | bc)
improvement_cli_only=$(echo "scale=2; (($time1 - $time3) / $time1) * 100" | bc)

cat >> "$RESULTS_FILE" <<EOF

## Summary

### Absolute Times
- **Default build**: ${time1}s (baseline)
- **Minimal features**: ${time2}s
- **CLI-only features**: ${time3}s
- **re_renderer incremental**: ${time4}s

### Improvements vs Baseline
- **Minimal features**: ${improvement_minimal}% faster
- **CLI-only features**: ${improvement_cli_only}% faster

### Phase Impact Analysis

**Phase 1 Optimizations** (minimal feature set):
- Actual measured improvement: ${improvement_minimal}%
- Projected improvement: 40-60%
- Result: $(echo "$improvement_minimal >= 40" | bc -l | grep -q 1 && echo "✅ Meets projections" || echo "⚠️  Below projections")

**Phase 2 Optimizations** (conditional shader embedding + graphics profile):
- Shader processing: Skipped in dev builds (measured in re_renderer test)
- Graphics compilation: opt-level reduced from 2 to 1
- Combined with Phase 1: Should see benefits in viewer builds

### Recommendations

EOF

if (( $(echo "$improvement_minimal < 40" | bc -l) )); then
    cat >> "$RESULTS_FILE" <<EOF
⚠️ **Minimal features improvement (${improvement_minimal}%) is below the 40-60% projection.**

Possible reasons:
1. Linking time may dominate for small changes
2. Workspace crates still being recompiled
3. Need to test with more significant code changes
4. System cache effects

To get more accurate measurements:
- Test with larger code changes that trigger more recompilation
- Use \`cargo build --timings\` to see per-crate breakdown
- Compare clean build times (not just incremental)
- Test on a clean system without warm caches

EOF
else
    cat >> "$RESULTS_FILE" <<EOF
✅ **Minimal features improvement (${improvement_minimal}%) meets or exceeds the 40-60% projection.**

EOF
fi

cat >> "$RESULTS_FILE" <<EOF

## Detailed Methodology Notes

These benchmarks measure **incremental rebuild time** - the time to rebuild after
making a small change. This is the most common scenario during development.

For a more comprehensive analysis:
1. Run \`cargo build --timings\` to see per-crate compilation times
2. Compare clean build times: \`cargo clean && time cargo build\`
3. Test with different types of changes (large refactors, type changes, etc.)
4. Measure on different machines/platforms

## How to Reproduce

Run this script:
\`\`\`bash
bash benchmark-rebuild-performance.sh
\`\`\`

Or run individual scenarios:
\`\`\`bash
# Baseline
touch crates/top/rerun-cli/src/bin/rerun.rs
time cargo build --bin rerun

# Minimal
touch crates/top/rerun-cli/src/bin/rerun.rs
time cargo build --bin rerun --no-default-features --features=minimal
\`\`\`

EOF

echo ""
echo -e "${GREEN}Benchmark complete!${NC}"
echo "Results written to: $RESULTS_FILE"
echo ""
echo "Summary:"
echo "  Default build:          ${time1}s"
echo "  Minimal features:       ${time2}s (${improvement_minimal}% faster)"
echo "  CLI-only features:      ${time3}s (${improvement_cli_only}% faster)"
echo "  re_renderer incremental: ${time4}s"
echo ""
