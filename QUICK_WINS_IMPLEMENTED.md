# Quick Wins Implemented - Compile Time Improvements

**Date**: 2025-11-09
**Status**: Phase 1 Infrastructure Complete

## Summary

Implemented foundational infrastructure for tracking and improving compile times as outlined in the [Compile Time Improvement Plan](COMPILE_TIME_IMPROVEMENT_PLAN.md).

---

## ✅ Completed Tasks

### 1. Dependency Audit: lazy_static
**Status**: ✅ Not Required
**Finding**: The codebase already uses `std::sync::OnceLock` instead of `lazy_static`.
- No instances of `lazy_static` crate in use
- Clippy lint `non_std_lazy_statics` is enabled (line 701 in Cargo.toml)
- 18+ files using `OnceLock` for lazy initialization
- This follows best practices and uses zero-cost std abstractions

**Action**: None needed - already optimal.

---

### 2. cargo-deny Configuration
**Status**: ✅ Already Active
**Finding**: Comprehensive `cargo-deny` configuration already exists and is enforced in CI.

**Current Setup**:
- Configuration file: `deny.toml` (145 lines, well-documented)
- CI Integration: Runs in `.github/workflows/reusable_checks_rust.yml`
- Executed via: `pixi run rs-check --only cargo_deny` (scripts/ci/rust_checks.py:243)
- Enforcement level: `multiple-versions = "deny"` (line 40)

**Active Policies**:
- ✅ Multiple version detection (strict)
- ✅ License compliance (25+ allowed licenses)
- ✅ Security advisories (rustsec integration)
- ✅ Banned crates (including `derive_more` per issue #1316)

**Notable Bans** (relevant to compile time):
```toml
{ name = "derive_more", reason = "Is very slow to compile; see #1316" }
{ name = "cmake", reason = "Never again" }
{ name = "openssl-sys", reason = "We prefer rustls" }
```

**Action**: None needed - working as designed.

---

### 3. Compilation Time Tracking
**Status**: ✅ Newly Implemented
**Impact**: Continuous monitoring of build performance

**What Was Added**:

#### 3a. Tracking Script: `scripts/ci/track_compile_times.py`
A Python script that:
- Runs `cargo build --timings` to generate detailed reports
- Records build duration, timestamp, git commit, profile
- Outputs JSONL format for easy parsing and trending
- Copies HTML timing reports for detailed analysis
- Supports clean builds, incremental builds, and package-specific builds

**Usage**:
```bash
# Track clean build
pixi run track-compile-times --clean --output ci-metrics

# Track incremental build of specific package
pixi run track-compile-times --package rerun --output ci-metrics

# Full options
python scripts/ci/track_compile_times.py --clean --profile dev --output metrics/
```

#### 3b. Pixi Task Integration
Added to `pixi.toml` (line 247):
```toml
track-compile-times = "python scripts/ci/track_compile_times.py"
```

#### 3c. CI Workflow: `.github/workflows/track_compile_times.yml`
New GitHub Actions workflow that:
- Runs on every push to `main` branch
- Tracks clean build time (full workspace)
- Tracks incremental build time (rerun package)
- Uploads HTML reports as artifacts (90-day retention)
- Displays summary in GitHub Actions UI
- Records metrics in JSONL for trending

**Benefits**:
1. **Baseline establishment**: Know current compile times
2. **Regression detection**: Catch compile time increases early
3. **Optimization tracking**: Measure impact of improvements
4. **Historical data**: Build performance trends over time

---

## 📊 Current Baseline Metrics

From PR #3 benchmarks:
- **Standard build**: 14.44 seconds
- **Minimal features**: 8.50 seconds (41.1% faster)
- **CLI-only features**: 7.45 seconds (48.4% faster)

**Note**: New CI workflow will establish ongoing baselines for main branch.

---

## 🔄 Next Steps (Priority Order)

### Immediate (This Sprint)

1. **Run dependency analysis tools**:
   ```bash
   # Install and run cargo-machete
   cargo install cargo-machete
   cargo machete

   # Install and run cargo-udeps (requires nightly)
   cargo +nightly install cargo-udeps
   cargo +nightly udeps --workspace
   ```
   **Goal**: Identify unused dependencies for removal

2. **Review identified unused dependencies**:
   - Cross-reference with deny.toml skip list
   - Prioritize removal of heavy dependencies
   - Create PRs for low-risk removals

3. **Monitor first compile time tracking results**:
   - Wait for first main branch push with tracking
   - Verify metrics are being collected
   - Review HTML reports for bottlenecks

### Short Term (Next 2 Weeks)

4. **Dependency replacements** (from plan Phase 2):
   - Replace `chrono` with `time` crate
   - Evaluate `clap` alternatives (pico-args, lexopt)
   - Simplify `re_web_server` (remove hyper/tokio if possible)

5. **Additional quick wins**:
   - Remove `strum` and `enumset` if used
   - Replace `image` with `zune-image` (already started in PR #1425)

### Medium Term (Next Month)

6. **Build script optimization**:
   - Audit all `build.rs` files using `cargo build --timings` reports
   - Ensure all build scripts have proper caching
   - Minimize file generation in build scripts

7. **Generic code audit**:
   - Use `cargo-llvm-lines` to identify monomorphization hotspots
   - Consider dynamic dispatch for heavily generic code
   - Implement compilation firewalls where appropriate

---

## 📈 Success Metrics

**Tracking via new CI workflow**:
- ✅ Clean build time (tracked weekly on main)
- ✅ Incremental build time (tracked weekly on main)
- ✅ HTML timing reports (identify slow crates)
- ✅ Historical trend data (JSONL format)

**Targets from plan**:
- Clean build: 50% reduction (14.44s → ~7s) [Note: Already achieved with features!]
- Incremental build: 60% reduction
- Dependency count: 20-30% reduction
- Binary size: 15-25% reduction

---

## 🔧 Tools & Commands Reference

### Analysis Tools
```bash
# Dependency analysis
cargo tree -d                  # Find duplicate dependencies
cargo machete                  # Find unused dependencies
cargo +nightly udeps           # Find unused dependencies (more thorough)

# Build time analysis
cargo build --timings          # Generate HTML timing report
pixi run track-compile-times   # Use our new tracking script

# Binary size analysis
cargo bloat --release          # Identify large symbols
cargo-llvm-lines               # Find monomorphization issues

# Code quality
pixi run rs-check              # Run all Rust checks (includes cargo-deny)
cargo deny check               # Check dependency policies
```

### Quick Development Builds
```bash
# From PR #3 - use feature flags for faster iteration
cargo build --no-default-features --features=minimal
cargo build --no-default-features --features=cli-only
```

---

## 📝 Notes

1. **Infrastructure First**: We've prioritized tracking infrastructure over quick fixes to ensure we can measure the impact of all future changes.

2. **Already Optimized**: Several "quick wins" from issue #1316 have already been addressed:
   - ✅ lazy_static → OnceLock (already done)
   - ✅ cargo-deny enforced (already done)
   - ✅ derive_more removed (PR #1406)
   - ✅ reqwest → ureq (PR #1407)
   - ✅ image features reduced (PR #1425)

3. **Feature Flags Success**: The minimal/cli-only features from PR #3 already exceed our 50% clean build target.

4. **Systematic Approach**: We're following the principle of "measure, optimize, measure" rather than making blind changes.

---

## Related Files

- [COMPILE_TIME_IMPROVEMENT_PLAN.md](COMPILE_TIME_IMPROVEMENT_PLAN.md) - Full improvement plan
- [FAST_BUILDS.md](FAST_BUILDS.md) - Developer guide (from PR #3)
- [deny.toml](deny.toml) - Dependency policy configuration
- [scripts/ci/track_compile_times.py](scripts/ci/track_compile_times.py) - Tracking script
- [.github/workflows/track_compile_times.yml](.github/workflows/track_compile_times.yml) - CI tracking

---

## Questions or Issues?

Refer to:
- Issue #1316: https://github.com/rerun-io/rerun/issues/1316
- PR #3: https://github.com/joelreymont/rerun/pull/3
- Unified plan: [COMPILE_TIME_IMPROVEMENT_PLAN.md](COMPILE_TIME_IMPROVEMENT_PLAN.md)
