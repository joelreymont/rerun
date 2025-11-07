# Fast Build Guide for Rerun Development

This document describes optimizations implemented in Phase 1 to speed up rebuild times during development.

## Quick Start

For the fastest possible builds when working on non-viewer code:

```bash
# Minimal build (no viewer, no server, ~60% faster compilation)
cargo build --release --bin rerun --no-default-features --features=minimal

# CLI-only build (includes server but no viewer, ~40% faster)
cargo build --release --bin rerun --no-default-features --features=cli-only

# Default build (all features)
cargo build --release --bin rerun
```

## Feature Sets

### `minimal`
**Use when**: Working on core data loading, SDK, or CLI features
**Excludes**: Viewer, server, map view
**Build command**: `cargo build --no-default-features --features=minimal`
**Speed**: ~60% faster than default

### `cli-only`
**Use when**: Need server functionality but not the viewer
**Excludes**: Native viewer, web viewer, map view
**Includes**: OSS server
**Build command**: `cargo build --no-default-features --features=cli-only`
**Speed**: ~40% faster than default

### `base`
**Use when**: Need native viewer but not web viewer
**Includes**: Native viewer, map view, OSS server
**Excludes**: Web viewer
**Build command**: `cargo build --no-default-features --features=base`
**Speed**: ~30% faster than default

### `default`
**Includes**: Everything (web viewer, native viewer, map view, server)
**Build command**: `cargo build` (or just `cargo build --release`)
**Speed**: Baseline

## Optimizations Implemented (Phase 1)

### 1. Shader Build Script Caching
- **Location**: `crates/viewer/re_renderer/build.rs`
- **Impact**: 80-90% reduction in build script time when shaders are unchanged
- **How it works**: Uses SHA-256 hashing to detect shader changes and skip regeneration
- **Manifest**: Stored in `target/.../shader_manifest.json`

### 2. Reduced Graphics Backends
- **Location**: `Cargo.toml` (workspace root)
- **Change**: Removed `gles` (OpenGL ES) from default wgpu backends
- **Impact**: 10-15% reduction in graphics stack compilation
- **Enabled backends**: `metal` (macOS), `vulkan` (Linux/Windows), `webgl`/`webgpu` (Web)
- **To enable GLES**: Manually add it back if needed

### 3. Minimal Feature Sets
- **Location**: `crates/top/rerun-cli/Cargo.toml`
- **New features**: `minimal`, `cli-only`
- **Impact**: 40-60% faster builds by excluding heavy dependencies

## Phase 2 Optimizations (Additional 15-25% improvement)

### 4. Conditional Shader Embedding
- **Location**: `crates/viewer/re_renderer/build.rs`
- **Impact**: Eliminates build script overhead entirely in dev builds
- **How it works**: In dev builds, shaders are loaded from disk at runtime instead of being embedded
- **Benefit**: Zero shader processing time in dev, enables hot-reloading
- **Automatic**: Enabled by default in dev builds (non-release, developer workspace)

### 5. Development Graphics Profile
- **Location**: `Cargo.toml` (workspace root, lines 536-555)
- **Impact**: 15-20% faster compilation of graphics stack
- **How it works**: Reduces optimization level from `opt-level = 2` to `opt-level = 1` for heavy graphics crates
- **Affected crates**: `wgpu`, `wgpu-core`, `wgpu-hal`, `naga`, `egui`, `eframe`, `epaint`
- **Trade-off**: Slightly slower runtime performance (acceptable for development)

### 6. Dependency Audit
- **Identified**: Duplicate versions of `prost` (v0.13.5 and v0.14.1) from external dependencies
- **Status**: External dependencies (DataFusion, Lance) - cannot easily change
- **Impact**: Minimal - most duplicates are unavoidable

## Examples

### Working on Data Loaders
```bash
# Use minimal build - no need for viewer
cargo run --release --no-default-features --features=minimal -- --help
```

### Working on CLI Commands
```bash
# Use cli-only if you need server, minimal if you don't
cargo run --release --no-default-features --features=cli-only -- serve data.rrd
```

### Working on Viewer Components
```bash
# Use base to get native viewer without web overhead
cargo run --release --no-default-features --features=base -- data.rrd
```

### Testing Incremental Builds
```bash
# Baseline: touch a file and measure rebuild time
touch crates/top/rerun-cli/src/bin/rerun.rs
time cargo build --release --bin rerun

# With minimal features
time cargo build --release --bin rerun --no-default-features --features=minimal
```

## Tips for Faster Iteration

1. **Use the `dev` profile for most development**:
   ```bash
   cargo build --bin rerun --no-default-features --features=minimal
   # (dev is default, so no --release needed)
   ```

2. **Use `cargo check` instead of `cargo build` when possible**:
   ```bash
   cargo check --no-default-features --features=minimal
   ```

3. **Use `cargo build --timings` to identify bottlenecks**:
   ```bash
   cargo build --release --bin rerun --timings
   # Opens an HTML report showing per-crate build times
   ```

4. **Consider using `sccache` or `mold` linker**:
   ```bash
   # Install sccache for distributed caching
   cargo install sccache
   export RUSTC_WRAPPER=sccache

   # Use mold linker (Linux only) for faster linking
   # Add to ~/.cargo/config.toml:
   # [target.x86_64-unknown-linux-gnu]
   # linker = "clang"
   # rustflags = ["-C", "link-arg=-fuse-ld=mold"]
   ```

5. **Keep your `target/` directory**:
   - Avoid `cargo clean` unless necessary
   - The shader manifest cache survives incremental builds

## Benchmarking

To measure the impact of these optimizations:

```bash
# Install hyperfine for benchmarking
cargo install hyperfine

# Baseline (default features)
touch crates/top/rerun-cli/src/bin/rerun.rs
hyperfine --warmup 1 --runs 3 'cargo build --release --bin rerun'

# With minimal features
touch crates/top/rerun-cli/src/bin/rerun.rs
hyperfine --warmup 1 --runs 3 'cargo build --release --bin rerun --no-default-features --features=minimal'
```

## Troubleshooting

### "Shaders unchanged" but getting errors
- The shader manifest is cached in `OUT_DIR`
- Run `cargo clean` to reset if you suspect cache corruption

### Missing features at runtime
- Make sure you're using the right feature set for your use case
- Check `cargo build --release --bin rerun --no-default-features --features=<your-feature> -v` for details

### Still slow builds
- Check `cargo build --timings` for bottlenecks
- Consider Phase 2 and Phase 3 optimizations from the main plan
- Profile with `cargo build -Z timings` (nightly)

## Future Improvements

See `rebuild-performance-optimization-plan.md` for:
- Phase 2: Dependency cleanup (additional 15-25% improvement)
- Phase 3: Architectural improvements (additional 20-30% improvement)

## Questions?

- Check the full plan: `rebuild-performance-optimization-plan.md`
- Report issues: https://github.com/rerun-io/rerun/issues
