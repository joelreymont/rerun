# Rerun CLI Rebuild Performance Optimization Plan

## Executive Summary

This document outlines a comprehensive plan to address rebuild performance issues in the rerun-cli binary, specifically targeting:
- **Issue 3**: Expensive Build Scripts
- **Issue 4**: Massive Dependency Graph
- **Issue 5**: Graphics Stack Compilation

**Estimated Impact**: Reduce incremental rebuild times by 40-60% for common development workflows.

---

## Issue 3: Expensive Build Scripts

### Current Problem
- `crates/viewer/re_renderer/build.rs` (252 lines) processes WGSL shader files on every rebuild
- Embeds all shaders into binary even when shaders haven't changed
- No caching mechanism for shader processing
- Runs even for builds that don't touch the renderer

### Proposed Solutions

#### 3.1 Add Build Script Caching (HIGH PRIORITY)
**Location**: `crates/viewer/re_renderer/build.rs`

**Action Items**:
1. Implement content-based hashing for shader files
2. Store hash manifest in `target/` directory
3. Skip processing if hashes match previous build
4. Use `rerun-if-changed` directives more precisely

**Implementation**:
```rust
// In build.rs
use std::collections::BTreeMap;
use std::fs;

fn compute_shader_hash(path: &Path) -> String {
    let content = fs::read(path).unwrap();
    format!("{:x}", blake3::hash(&content))
}

fn load_previous_manifest() -> BTreeMap<PathBuf, String> {
    // Load from target/shader_manifest.json
}

fn should_rebuild_shaders(current: &BTreeMap<PathBuf, String>) -> bool {
    let previous = load_previous_manifest();
    current != &previous
}
```

**Expected Impact**: 80-90% reduction in build script execution time for unchanged shaders

#### 3.2 Conditional Shader Embedding (MEDIUM PRIORITY)
**Location**: `crates/viewer/re_renderer/build.rs`, `crates/viewer/re_renderer/Cargo.toml`

**Action Items**:
1. Add feature flag `embed-shaders` (enabled by default in release)
2. In dev builds, load shaders from filesystem at runtime
3. Only embed shaders when building for distribution

**Implementation**:
```toml
# In re_renderer/Cargo.toml
[features]
default = ["embed-shaders"]
embed-shaders = []
```

**Expected Impact**: Eliminates build script overhead in dev builds

#### 3.3 Parallelize Shader Processing (LOW PRIORITY)
**Location**: `crates/viewer/re_renderer/build.rs`

**Action Items**:
1. Use `rayon` to process shaders in parallel
2. Batch shader processing operations
3. Profile to identify bottlenecks

**Expected Impact**: 30-50% reduction in build script time when it does run

---

## Issue 4: Massive Dependency Graph

### Current Problem
- 2,226 total unique dependencies
- 84 workspace crates (many interdependent)
- 20+ DataFusion sub-crates pulled in by default
- 12+ Arrow crates required by storage
- Flat dependency structure causes cascading rebuilds

### Proposed Solutions

#### 4.1 Dependency Audit and Pruning (HIGH PRIORITY)
**Locations**: All `Cargo.toml` files

**Action Items**:
1. Run `cargo tree --duplicates` to find duplicate dependencies
2. Run `cargo udeps` to find unused dependencies
3. Consolidate versions of commonly used crates
4. Remove dependencies that can be replaced with std library

**Commands**:
```bash
# Install tools
cargo install cargo-tree
cargo install cargo-udeps

# Audit
cargo tree --duplicates > duplicate_deps.txt
cargo +nightly udeps --all-targets > unused_deps.txt

# Analyze
rg "duplicate" duplicate_deps.txt
```

**Expected Impact**: 10-15% reduction in total dependency count

#### 4.2 Feature-Gate Heavy Dependencies (HIGH PRIORITY)
**Locations**:
- `crates/top/rerun-cli/Cargo.toml`
- `crates/top/rerun/Cargo.toml`
- `crates/store/re_dataframe/Cargo.toml`
- `crates/store/re_datafusion/Cargo.toml`

**Action Items**:
1. Make DataFusion optional (currently required by default)
2. Create `minimal-cli` feature set
3. Split server dependencies from core CLI

**Implementation**:
```toml
# In rerun-cli/Cargo.toml
[features]
default = ["web_viewer", "base"]
base = ["native_viewer", "map_view", "oss_server"]
minimal = [] # No viewer, no server, just core functionality
cli-only = ["data_loaders"] # CLI operations without viewer

[dependencies]
re_dataframe = { workspace = true, optional = true }
re_datafusion = { workspace = true, optional = true }
```

**Expected Impact**: 30-40% faster builds when using minimal features

#### 4.3 Split Monolithic Crates (MEDIUM PRIORITY)
**Locations**:
- `crates/viewer/re_viewer/Cargo.toml` (183 lines, depends on all 32 viewer crates)
- `crates/store/re_entity_db/Cargo.toml`

**Action Items**:
1. Split `re_viewer` into `re_viewer_core` and `re_viewer_ui`
2. Create `re_viewer_plugins` crate for optional viewers
3. Make individual view crates truly optional

**Architecture**:
```
re_viewer_core (minimal viewer infrastructure)
├── re_viewer_ui (UI components)
└── re_viewer_plugins (optional)
    ├── re_view_spatial
    ├── re_view_graph
    └── re_view_map
```

**Expected Impact**: 20-30% reduction in rebuild scope for viewer changes

#### 4.4 Workspace Dependency Optimization (MEDIUM PRIORITY)
**Location**: Root `Cargo.toml`

**Action Items**:
1. Use `workspace.dependencies` more aggressively
2. Ensure all version numbers are unified
3. Reduce inter-crate dependencies where possible

**Expected Impact**: Faster Cargo dependency resolution (5-10% build time)

---

## Issue 5: Graphics Stack Compilation

### Current Problem
- wgpu v27.0.1 compiles multiple backend implementations (Vulkan, Metal, DX12)
- All backends compile even when only one is used
- egui v0.33.0 + eframe dependencies are heavy
- 32 viewer component crates all pull in graphics dependencies

### Proposed Solutions

#### 5.1 Feature-Gate Graphics Backends (HIGH PRIORITY)
**Locations**:
- `crates/viewer/re_renderer/Cargo.toml`
- Root `Cargo.toml` (wgpu features)

**Action Items**:
1. Make wgpu backends conditional on platform
2. Only compile the backend needed for current platform
3. Add `graphics-minimal` feature for development

**Implementation**:
```toml
# In re_renderer/Cargo.toml
[features]
default = ["graphics-full"]
graphics-full = ["wgpu/vulkan", "wgpu/metal", "wgpu/dx12"]
graphics-minimal = [] # Use wgpu defaults only
graphics-vulkan = ["wgpu/vulkan"]
graphics-metal = ["wgpu/metal"]
graphics-dx12 = ["wgpu/dx12"]

# Platform-specific defaults
[target.'cfg(target_os = "linux")'.dependencies]
wgpu = { workspace = true, features = ["vulkan"] }

[target.'cfg(target_os = "macos")'.dependencies]
wgpu = { workspace = true, features = ["metal"] }

[target.'cfg(target_os = "windows")'.dependencies]
wgpu = { workspace = true, features = ["dx12"] }
```

**Expected Impact**: 25-35% reduction in graphics stack compilation time

#### 5.2 Lazy Viewer Component Loading (MEDIUM PRIORITY)
**Locations**:
- `crates/viewer/re_viewer/src/lib.rs`
- Individual view crates

**Action Items**:
1. Convert viewer components to dynamic plugins
2. Lazy-load components only when needed
3. Use dynamic linking for development builds

**Implementation**:
```rust
// Runtime plugin loading
pub trait ViewerPlugin {
    fn name(&self) -> &str;
    fn render(&mut self, ctx: &ViewerContext);
}

lazy_static! {
    static ref PLUGINS: RwLock<HashMap<String, Box<dyn ViewerPlugin>>> =
        RwLock::new(HashMap::new());
}
```

**Expected Impact**: 40-50% reduction in viewer rebuild time

#### 5.3 Optimize egui Compilation (LOW PRIORITY)
**Location**: Root `Cargo.toml`

**Action Items**:
1. Review egui features and disable unused ones
2. Consider egui version with faster compile times
3. Profile egui compilation to identify bottlenecks

**Expected Impact**: 10-15% reduction in UI stack compilation

#### 5.4 Development Graphics Profile (MEDIUM PRIORITY)
**Location**: Root `Cargo.toml`

**Action Items**:
1. Create a `dev-graphics` profile with reduced optimization for graphics crates
2. Allow faster iteration on non-graphics code

**Implementation**:
```toml
[profile.dev.package.wgpu]
opt-level = 1  # Reduce from 2
debug = true

[profile.dev.package.egui]
opt-level = 1  # Reduce from 2
debug = true
```

**Expected Impact**: 15-20% faster dev builds (with slight runtime performance cost)

---

## Implementation Strategy

### Phase 1: Quick Wins (Week 1-2)
**Estimated Impact**: 30-40% rebuild time reduction

1. **Implement shader build script caching** (Issue 3.1)
   - Highest impact, lowest risk
   - Can be done without API changes

2. **Feature-gate graphics backends** (Issue 5.1)
   - Significant compilation reduction
   - No breaking changes

3. **Add minimal feature set** (Issue 4.2)
   - Enables developers to opt-out of heavy features
   - Backward compatible

### Phase 2: Dependency Cleanup (Week 3-4)
**Estimated Impact**: Additional 15-25% improvement

1. **Dependency audit and pruning** (Issue 4.1)
   - Remove unused dependencies
   - Consolidate duplicate versions

2. **Conditional shader embedding** (Issue 3.2)
   - Eliminate build script overhead in dev
   - Requires runtime shader loading path

3. **Development graphics profile** (Issue 5.4)
   - Fine-tune optimization levels
   - Balance compile time vs runtime performance

### Phase 3: Architectural Improvements (Week 5-8)
**Estimated Impact**: Additional 20-30% improvement

1. **Split monolithic crates** (Issue 4.3)
   - Requires careful API design
   - Breaking changes, needs major version bump

2. **Lazy viewer component loading** (Issue 5.2)
   - Significant architecture change
   - Enables true plugin system

3. **Parallelize shader processing** (Issue 3.3)
   - Polish build script performance

---

## Measurement and Validation

### Benchmarking Setup
```bash
# Baseline measurement
cargo clean
hyperfine --warmup 1 --runs 3 \
  'cargo build --release --bin rerun'

# Incremental rebuild (touch a file)
touch crates/top/rerun-cli/src/bin/rerun.rs
hyperfine --warmup 1 --runs 5 \
  'cargo build --release --bin rerun'
```

### Success Metrics
- **Clean build time**: Maintain or improve (not critical)
- **Incremental rebuild (CLI change)**: < 10 seconds (currently ~30s)
- **Incremental rebuild (viewer change)**: < 60 seconds (currently ~3-5 minutes)
- **Dependency count**: Reduce by 15-20%
- **Developer satisfaction**: Survey feedback

### Monitoring
1. Add CI job to track build times over time
2. Track per-crate compilation times with `cargo build --timings`
3. Monthly dependency audit

---

## Risks and Mitigation

### Risk 1: Feature Fragmentation
**Impact**: Users build with wrong feature set and miss functionality

**Mitigation**:
- Keep sensible defaults
- Clear documentation on feature flags
- CI tests for all feature combinations

### Risk 2: Runtime Performance Regression
**Impact**: Dev builds become too slow to use

**Mitigation**:
- Profile before and after changes
- Keep opt-level = 2 for critical crates (video decoder, renderer hot paths)
- Make aggressive optimization opt-in

### Risk 3: Maintenance Burden
**Impact**: More complex build configuration

**Mitigation**:
- Document all feature flags clearly
- Automate feature combination testing in CI
- Keep feature matrix manageable

---

## References

### Key Files
- `crates/top/rerun-cli/Cargo.toml:44-47` - Default features
- `crates/viewer/re_renderer/build.rs` - Shader build script
- `Cargo.toml:425-549` - Profile configurations
- `Cargo.toml:528-529` - External dependency optimization

### Tools
- `cargo tree` - Dependency analysis
- `cargo udeps` - Unused dependency detection
- `cargo build --timings` - Build time profiling
- `hyperfine` - Benchmarking
- `cargo-feature-graph` - Feature visualization

### Further Reading
- [The Cargo Book - Features](https://doc.rust-lang.org/cargo/reference/features.html)
- [wgpu Feature Flags](https://docs.rs/wgpu/latest/wgpu/#feature-flags)
- [Fast Rust Builds](https://matklad.github.io/2021/09/04/fast-rust-builds.html)
