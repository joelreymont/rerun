# Quarter 2 Dependency Analysis - Compile Time Optimization

**Date**: 2025-11-09
**Status**: Analysis Complete

## Executive Summary

Comprehensive analysis of Quarter 2 dependency optimization opportunities from the [Compile Time Improvement Plan](COMPILE_TIME_IMPROVEMENT_PLAN.md). Most proposed "easy wins" from Issue #1316 are either already done, not applicable, or would have high implementation risk relative to compile time benefit.

**Key Finding**: The low-hanging fruit has already been picked. Remaining optimizations require careful evaluation of risk vs. reward.

---

## Dependency Analysis Results

### 1. chrono → time ❌ Not Recommended

**Current Usage**:
- Workspace dependency in `Cargo.toml` line 207
- Used by: `re_datafusion`, `re_redap_tests`, `re_server`
- Actual code usage: Only 1 file (`rerun_py/src/catalog/entry.rs`)

**Analysis**:
```toml
# Cargo.toml:207
chrono = { version = "0.4.42", default-features = false }
# Comment: "Needed for datafusion"
```

**Decision**: ❌ **Do Not Replace**

**Rationale**:
1. Minimal direct usage (1 file)
2. Required by external dependency (datafusion)
3. Replacing would not eliminate the transitive dependency
4. Risk: Breaking datafusion integration
5. Compile time impact: **Negligible** (already pulled in transitively)

---

### 2. clap → pico-args/lexopt ❌ Not Recommended

**Current Usage**:
- Workspace dependency: `clap = "4.5"`
- **Core crates**: `rerun`, `re_server`, `re_perf_telemetry`
- **Examples**: 10+ Rust examples use clap
- **Tests**: 4+ test binaries use clap

**Features Used**: `derive` (proc macros), `env`

**Analysis**:
Clap is deeply integrated for CLI argument parsing with derive macros:
```rust
#[derive(clap::Parser)]
struct Args {
    #[clap(long)]
    option: String,
}
```

**Decision**: ❌ **Do Not Replace** (for now)

**Rationale**:
1. **High integration cost**: Would require rewriting 15+ crates
2. **Derive macros**: Heavy use of `#[derive(Parser)]` - manual replacement would add LOC
3. **Compilation impact**: Modern clap 4.x is much faster than older versions
4. **Stability risk**: High chance of introducing CLI parsing bugs
5. **Benefit**: Estimated 5-10% reduction in affected crates only

**Alternative**: Mark as "Future consideration" if compile times remain an issue after other optimizations.

---

### 3. Simplify re_web_server (remove hyper/tokio) ❌ Not Applicable

**Finding**: There is **no `re_web_server` crate** in the repository.

**Actual Web Server**: `re_server` crate exists at `crates/store/re_server/`

**Dependencies**:
```toml
# re_server/Cargo.toml
tokio = { features = ["rt-multi-thread", "macros", "signal"] }
tonic = { workspace = true }  # gRPC framework (uses tokio)
tonic-web = { workspace = true }
tower = { workspace = true }
tower-http = { workspace = true }
```

**Analysis**:
- `re_server` is a gRPC server using tonic (which requires tokio)
- Tokio used in 13+ crates across the workspace
- Removing tokio would break: gRPC, async data processing, dataframe engine

**Decision**: ❌ **Not Applicable / Not Recommended**

**Rationale**:
1. Issue #1316 comment may be outdated or referred to different code
2. Current architecture requires async runtime for gRPC
3. Tokio is industry-standard and well-optimized
4. Replacing would require architectural redesign
5. Risk: **Very High** - would affect core functionality

---

### 4. strum → Manual implementations ⚠️ Possible (Low Priority)

**Current Usage**:
```toml
# Workspace: Cargo.toml:343
strum = { version = "0.26.3", features = ["derive"] }
# Comment: "need to update re_rav1d first"
```

**Used by**:
1. `rerun_py` - Python bindings
2. `re_dataframe_ui` - UI filters
3. `re_ui` - Command palette, color tables
4. `re_viewer` - Viewer commands

**Code usage**: 9 files total

**What strum provides**:
```rust
#[derive(strum::EnumIter, strum::Display)]
enum MyEnum { Variant1, Variant2 }
```

**Decision**: ⚠️ **Possible but Low Priority**

**Compile Time Impact**: Moderate (proc macros add compile time)
**Implementation Effort**: Moderate (manual trait implementations)
**Risk**: Low-Medium (straightforward refactoring)

**Recommendation**: Consider for Q3 if other optimizations show diminishing returns.

**Manual Alternative**:
```rust
// Instead of strum::EnumIter
impl MyEnum {
    fn iter() -> impl Iterator<Item = Self> {
        [Self::Variant1, Self::Variant2].into_iter()
    }
}
```

---

### 5. enumset → bitflags/manual ✅ Tractable

**Current Usage**:
```toml
# Workspace: Cargo.toml:237
enumset = "1.1.10"
```

**Used by**: **Only `re_renderer`** (`crates/viewer/re_renderer/`)

**Code usage**: 5 files
- `renderer/point_cloud.rs`
- `renderer/lines.rs`
- `renderer/mesh_renderer.rs`
- `draw_phases/mod.rs`
- `draw_phases/draw_phase_manager.rs`

**What enumset provides**:
```rust
#[derive(EnumSetType)]
enum DrawPhase { Opaque, Transparent, Overlay }

let phases: EnumSet<DrawPhase> = DrawPhase::Opaque | DrawPhase::Transparent;
```

**Decision**: ✅ **Tractable Replacement**

**Compile Time Impact**: Low-Moderate (proc macros, limited scope)
**Implementation Effort**: Low (5 files, single crate)
**Risk**: Low (isolated to renderer)

**Alternatives**:
1. **bitflags**: Standard, zero-cost abstraction
   ```rust
   bitflags::bitflags! {
       struct DrawPhases: u32 {
           const OPAQUE = 0b001;
           const TRANSPARENT = 0b010;
           const OVERLAY = 0b100;
       }
   }
   ```

2. **Manual implementation**: Custom set type with array/bitset

**Recommendation**: ✅ **Good candidate for Q2 implementation**

---

### 6. image → zune-image ✅ Already Started

**Status**: ✅ **Partially Completed** (PR #1425)

PR #1425: "Removed extraneous `image` features"

**Remaining Work**: Evaluate full replacement with `zune-image`

**Decision**: ✅ **Continue investigating**

**Next Steps**:
1. Audit current image crate usage
2. Test zune-image compatibility
3. Benchmark decode performance
4. Gradual migration if beneficial

---

## Build Script Audit

**Analyzed**: 19 build scripts across the workspace

### Findings: ✅ Already Optimized

**Categories**:

1. **Simple build info exports** (13 scripts):
   ```rust
   fn main() {
       re_build_tools::export_build_info_vars_for_crate("crate_name");
   }
   ```
   - **Impact**: Negligible (just sets env vars)
   - **Optimization**: None needed

2. **Shader embedding** (`re_renderer/build.rs`):
   - ✅ Hash-based caching already implemented
   - ✅ Skip in release/CI environments
   - ✅ Conditional for dev builds only
   - **Status**: Well-optimized

3. **Codegen with caching** (`re_types_builder/build.rs`):
   - ✅ Hash-based skip logic (lines 59-68)
   - ✅ Only runs when input files change
   - ✅ Only for developers who edit `.fbs` files
   - **Status**: Excellent caching strategy

4. **Feature detection** (`re_ui/build.rs`, `re_video/build.rs`):
   - Simple cfg detection, no file I/O
   - **Impact**: Negligible

### Conclusion: ✅ No build script optimizations needed

All build scripts follow best practices:
- Hash-based caching where needed
- Skip in CI/release when appropriate
- Minimal file system operations
- Use `write_file_if_necessary` to avoid unnecessary rebuilds

---

## Additional Findings

### Already Optimized (from previous PRs)

✅ **Completed** (no further action):
- `derive_more` → removed (PR #1406)
- `reqwest` → `ureq` (PR #1407)
- `image` features reduced (PR #1425)
- `lazy_static` → `OnceLock` (already using std)
- `cargo-deny` enforced (already in CI)

### Not Found / Not Applicable

❌ **Not in codebase**:
- `re_web_server` crate (doesn't exist)
- `hyper` dependency (not in workspace)
- Excessive use of `lazy_static` (already migrated)

---

## Recommendations

### Immediate Actions (Q2)

1. ✅ **Replace `enumset` in re_renderer**
   - **Effort**: Low (5 files, 1 crate)
   - **Impact**: Small but measurable
   - **Risk**: Low
   - **Alternative**: bitflags or manual implementation

### Future Consideration (Q3)

2. ⚠️ **Evaluate `strum` removal**
   - **Effort**: Moderate (9 files, 4 crates)
   - **Impact**: Moderate (remove proc macro overhead)
   - **Risk**: Low-Medium
   - **Prerequisite**: Update re_rav1d first (per workspace comment)

3. ⚠️ **Consider `clap` alternatives**
   - **Only if**: Compile times remain critical issue
   - **Effort**: High (15+ crates)
   - **Impact**: 5-10% in CLI crates
   - **Risk**: Medium (potential for bugs)

### Not Recommended

❌ **Do NOT pursue**:
- `chrono` replacement (needed transitively)
- `tokio` removal (architectural requirement)
- Simplifying non-existent `re_web_server`

---

## Revised Q2 Roadmap

Based on this analysis, updating the implementation plan:

### Quarter 2: Revised Scope

**High Value, Low Risk**:
- [x] Dependency analysis (this document)
- [ ] Replace `enumset` with `bitflags` in re_renderer
- [ ] Complete `image` → `zune-image` evaluation
- [ ] Add dependency tracking to CI (alert on increases)

**Deferred to Q3**:
- [ ] Evaluate `strum` removal (after re_rav1d update)
- [ ] Monomorphization analysis with `cargo-llvm-lines`
- [ ] Generic code audit

**Not Pursuing**:
- ~~Replace `chrono`~~ (not beneficial)
- ~~Replace `clap`~~ (too invasive for benefit)
- ~~Simplify web server~~ (not applicable)

---

## Expected Impact

### Conservative Estimates

**enumset removal**:
- **Scope**: 1 crate (re_renderer)
- **Savings**: 1-3% in re_renderer build time
- **Overall**: <1% workspace build time

**image → zune-image** (if pursued):
- **Scope**: Multiple crates using image decoding
- **Savings**: 5-10% in affected crates
- **Overall**: 2-5% workspace build time

**Total Q2 Impact**: 2-6% additional improvement

**Note**: This is on top of the 41-48% already achieved in Q1.

---

## Tools & Commands Used

```bash
# Dependency searches
grep -r "^chrono\s*=" --include="Cargo.toml"
grep -r "use chrono" --include="*.rs"

# Usage analysis
grep -r "strum::" --include="*.rs"
grep -r "enumset::" --include="*.rs"

# Build script audit
find . -name "build.rs" -type f

# Dependency tree analysis
cargo tree -p re_server
cargo tree -i chrono
```

---

## Next Steps

1. ✅ **Review this analysis** with team
2. **Decide**: Proceed with enumset replacement?
3. **Evaluate**: Is 2-6% additional gain worth the effort?
4. **Consider**: Focus on Q3 monomorphization instead?
5. **Update**: Revise COMPILE_TIME_IMPROVEMENT_PLAN.md based on findings

---

## References

- [COMPILE_TIME_IMPROVEMENT_PLAN.md](COMPILE_TIME_IMPROVEMENT_PLAN.md)
- [QUICK_WINS_IMPLEMENTED.md](QUICK_WINS_IMPLEMENTED.md)
- [Issue #1316](https://github.com/rerun-io/rerun/issues/1316)
- [PR #3](https://github.com/joelreymont/rerun/pull/3)

---

**Conclusion**: The original Issue #1316 suggestions were made before significant optimization work. Most recommendations are no longer applicable or would have poor risk/reward ratio. Focus should shift to Phase 3-4 optimizations (build system tuning, monomorphization analysis) for better returns.
