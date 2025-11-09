# Unified Compile Time Improvement Plan

## Executive Summary

This plan consolidates ongoing compile time optimization efforts, combining:
- **PR #3**: Shader caching, feature flags, and graphics backend optimization (41-48% improvements measured)
- **Issue #1316**: Dependency reduction, library replacements, and build tooling improvements

**Target**: 60-95% rebuild time improvement through comprehensive multi-phase approach

**📊 Implementation Status**: See [QUICK_WINS_IMPLEMENTED.md](QUICK_WINS_IMPLEMENTED.md) for Phase 1 progress report.

---

## Phase 1: Foundation (Completed/In Progress via PR #3)

### 1.1 Shader Build Caching ✅
**Status**: Implemented
**Impact**: 80-90% reduction in build script time
**Approach**: SHA-256 content hashing to prevent unnecessary shader regeneration

```rust
// Cache shaders based on content hash, not timestamps
let shader_hash = sha256(&shader_content);
if !cache.changed(shader_hash) { skip_rebuild(); }
```

### 1.2 Graphics Backend Reduction ✅
**Status**: Implemented
**Impact**: 10-15% compilation savings
**Change**: Removed OpenGL ES from default backends, retained only:
- Vulkan (primary)
- Metal (macOS)
- DX12 (Windows)
- WebGPU (web)

### 1.3 Feature Flags ✅
**Status**: Implemented
**Impact**: 40-60% faster for targeted builds
**New Features**:
- `minimal`: Core functionality only (41.1% faster)
- `cli-only`: Command-line tools without GUI (48.4% faster)

**Usage**:
```bash
cargo build --no-default-features --features=minimal
cargo build --no-default-features --features=cli-only
```

### 1.4 Developer Documentation ✅
**Status**: Created `FAST_BUILDS.md`
**Content**: Best practices for fast iteration during development

---

## Phase 2: Dependency Optimization

### 2.1 High-Impact Dependency Replacements

#### Priority 1: Large Transitive Dependencies

| Current | Replacement | Rationale | Estimated Impact |
|---------|------------|-----------|------------------|
| `clap` | `pico-args` or `lexopt` | Lighter CLI parsing, fewer proc macros | 15-20% reduction in CLI crate trees |
| `chrono` | `time` | Smaller, actively maintained, no legacy code | 5-10% reduction |
| `image` | `zune-image` | Faster, smaller, already removed excess features | 10-15% reduction |
| `sha2` | `blake3` or simple hasher | Overkill for build caching needs | 3-5% reduction |

#### Priority 2: Async Runtime Simplification

**Target**: `re_web_server` crate
- **Remove**: `hyper` + `tokio` (massive dependency trees)
- **Replace with**: Lightweight alternatives
  - Option A: `tiny_http` (zero dependencies)
  - Option B: `rouille` (minimal dependencies)
  - Option C: Simplify to basic HTTP if full features not needed

**Impact**: 20-30% reduction in web server compilation time

#### Priority 3: Macro Crate Elimination

| Crate | Action | Rationale |
|-------|--------|-----------|
| `derive_more` | ✅ Removed (PR #1406) | Manual trait implementations |
| `strum` | Remove | Replace with manual enum implementations or `enumn` |
| `enumset` | Simplify or remove | Use standard collections or bitflags |
| `lazy_static` | Replace with `once_cell` or `std::sync::OnceLock` | Lighter, std alternative available |

### 2.2 Dependency Audit & Enforcement

#### Immediate Actions
```bash
# Identify unused dependencies
cargo machete
cargo udeps

# Find duplicate crates in tree
cargo tree -d

# Enforce dependency policies
cargo deny check
```

#### Continuous Monitoring
- **Setup**: Add `cargo-deny` to CI pipeline
- **Policy**: Zero tolerance for duplicate major versions
- **Tracking**: Monthly dependency audit reports

---

## Phase 3: Build System & Compiler Optimizations

### 3.1 Conditional Compilation Strategies

#### Development Profile Optimization
```toml
# .cargo/config.toml or Cargo.toml
[profile.dev.package."wgpu*"]
opt-level = 1  # Light optimization for graphics stack

[profile.dev.package."egui*"]
opt-level = 1

[profile.dev]
incremental = true
codegen-units = 256  # Maximize parallelism
```

#### Conditional Shader Embedding
```rust
#[cfg(not(debug_assertions))]
const SHADERS: &[u8] = include_bytes!("compiled_shaders.bin");

#[cfg(debug_assertions)]
fn load_shaders() -> Vec<u8> {
    std::fs::read("shaders/").unwrap()  // Runtime loading
}
```

### 3.2 Workspace Optimization

#### Split Large Crates
- **Current Issue**: Monolithic crates block parallelization
- **Strategy**:
  - Identify crates >50k lines
  - Split into logical sub-crates where beneficial
  - Balance granularity vs. overhead

#### Build Script Minimization
- **Audit**: Review all `build.rs` files
- **Optimize**: Ensure caching, avoid unnecessary file generation
- **Measure**: Use `cargo build --timings` to identify slow build scripts

---

## Phase 4: Tooling & Metrics

### 4.1 Compilation Time Tracking

#### Automated Benchmarking
```bash
# Add to CI pipeline
cargo clean
time cargo build --timings -p rerun > build_metrics.txt

# Track over time (similar to performance benchmarks)
# Store results in git or external DB
```

#### Key Metrics to Track
- Total build time (clean & incremental)
- Per-crate compilation time
- Build script execution time
- Dependency count over time
- Binary size trends

### 4.2 Analysis Tools Integration

| Tool | Purpose | Usage |
|------|---------|-------|
| `cargo build --timings` | Identify slow crates | CI + local dev |
| `cargo tree` | Visualize dependencies | Manual audits |
| `cargo-bloat` | Binary size analysis | Release builds |
| `cargo-llvm-lines` | Code generation metrics | Identify monomorphization issues |
| `cargo-udeps` | Find unused deps | Monthly audits |
| `cargo-machete` | Detect unused deps | Pre-PR checks |

---

## Phase 5: Advanced Optimizations

### 5.1 Generic Code & Monomorphization

#### Problem
Excessive generic instantiation increases compile time and binary size.

#### Solutions
- **Dynamic dispatch where appropriate**: Replace generic parameters with trait objects
- **Type erasure patterns**: Hide generics behind concrete APIs
- **Compilation firewall**: Use opaque types to limit monomorphization

```rust
// Before: Many instantiations
fn process<T: Component>(data: T) { ... }

// After: Single implementation
fn process_erased(data: &dyn Component) { ... }
```

### 5.2 Compression Library Optimization

**Context**: Issue mentions puffin library compression
- **Investigate**: Current compression algorithm
- **Alternatives**: Try faster algorithms (lz4, zstd-fast modes)
- **Tradeoff**: Speed vs. compression ratio for dev builds

### 5.3 Parallel Compilation Tuning

```bash
# Experiment with codegen-units
CARGO_PROFILE_DEV_CODEGEN_UNITS=256 cargo build

# Use mold linker (Linux) or lld
[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=mold"]
```

---

## Implementation Roadmap

### Quarter 1: Foundation & Quick Wins
- [x] Shader caching (PR #3)
- [x] Feature flags (PR #3)
- [x] Graphics backend reduction (PR #3)
- [x] Remove `derive_more` (PR #1406)
- [x] Replace `reqwest` with `ureq` (PR #1407)
- [x] Remove image features (PR #1425)
- [x] Replace `lazy_static` with `once_cell` ✅ Already using OnceLock
- [x] Setup `cargo-deny` in CI ✅ Already configured and enforced
- [x] Add compilation time tracking to CI ✅ Implemented 2025-11-09

### Quarter 2: Dependency Optimization
- [ ] Replace `chrono` with `time`
- [ ] Evaluate and replace `clap` (if beneficial)
- [ ] Simplify `re_web_server` (remove hyper/tokio)
- [ ] Remove `strum` and `enumset`
- [ ] Replace `image` with `zune-image`
- [ ] Audit all build scripts
- [ ] Run `cargo-udeps` and `cargo-machete`

### Quarter 3: Build System Refinement
- [ ] Conditional shader embedding
- [ ] Development profile optimization
- [ ] Evaluate crate splitting opportunities
- [ ] Optimize compression in puffin
- [ ] Generic code audit

### Quarter 4: Measurement & Iteration
- [ ] Monthly compilation metrics review
- [ ] Dependency count enforcement
- [ ] Monomorphization analysis
- [ ] Linker optimization experiments
- [ ] Document final improvements

---

## Success Metrics

### Primary KPIs
- **Clean build time**: Target 50% reduction
- **Incremental build time**: Target 60% reduction
- **Dependency count**: Reduce by 20-30%
- **Binary size**: 15-25% reduction

### Already Achieved (PR #3)
- ✅ Minimal features: **41.1% faster** (14.44s → 8.50s)
- ✅ CLI-only features: **48.4% faster** (14.44s → 7.45s)

### Tracking
- Baseline measurements before each phase
- Weekly incremental build time monitoring
- Monthly dependency audits
- Quarterly comprehensive reviews

---

## Risk Mitigation

### Compatibility Concerns
- **Approach**: Feature flags maintain backward compatibility
- **Testing**: Ensure all feature combinations work
- **Documentation**: Clear migration guides for breaking changes

### Dependency Replacement Risks
- **Validation**: Thorough testing before replacement
- **Rollback plan**: Keep old dependencies initially as features
- **Incremental**: One replacement at a time with measurement

### Team Impact
- **Communication**: Document all changes in `FAST_BUILDS.md`
- **Training**: Share best practices for fast iteration
- **Feedback**: Regular check-ins on developer experience

---

## Resources & References

### Tools
- [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) - Dependency policy enforcement
- [cargo-machete](https://github.com/bnjbvr/cargo-machete) - Unused dependency detection
- [cargo-udeps](https://github.com/est31/cargo-udeps) - Unused dependency finder
- [cargo-bloat](https://github.com/RazrFalcon/cargo-bloat) - Binary size analysis
- [cargo-llvm-lines](https://github.com/dtolnay/cargo-llvm-lines) - LLVM IR analysis

### Related Issues & PRs
- Issue #1316: Improve compile times (tracking issue)
- PR #3: Rebuild performance optimization plan
- PR #1406: Remove derive_more
- PR #1407: Replace reqwest with ureq
- PR #1425: Remove image features

### Further Reading
- [The Rust Performance Book](https://nnethercote.github.io/perf-book/compile-times.html)
- [Fast Rust Builds](https://matklad.github.io/2021/09/04/fast-rust-builds.html)
- [Optimizing Rust Compile Times](https://endler.dev/2020/rust-compile-times/)

---

## Next Steps

1. **Review this plan** with the team for alignment
2. **Prioritize** specific dependency replacements based on impact vs. effort
3. **Establish baselines** for all key metrics
4. **Implement** quick wins from Quarter 1 checklist
5. **Setup** automated tracking infrastructure
6. **Iterate** based on measurements and feedback

**Last Updated**: 2025-11-09
**Status**: Active Development
**Owner**: @joelreymont
