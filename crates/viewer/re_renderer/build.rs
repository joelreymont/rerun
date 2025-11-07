//! This build script implements the second half of our cross-platform shader #import system.
//! The first half can be found in `src/file_resolver.rs`.
//!
//! It finds all WGSL shaders defined anywhere within `re_renderer`, and embeds them
//! directly into the released artifact for our `re_renderer` library.
//!
//! At run-time, for release builds only, those shaders will be available through an hermetic
//! virtual filesystem.
//! To the user, it will look like business as usual.
//!
//! See `re_renderer/src/workspace_shaders.rs` for the end result.

// TODO(cmc): this should only run for release builds

#![allow(clippy::allow_attributes, clippy::disallowed_types)] // False positives for using files on Wasm
#![expect(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail, ensure};
use sha2::{Digest, Sha256};
use walkdir::{DirEntry, WalkDir};

use re_build_tools::{
    Environment, get_and_track_env_var, rerun_if_changed, write_file_if_necessary,
};

// ---

/// A pre-parsed import clause, as in `#import <something>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportClause {
    /// The path being imported, as-is: neither canonicalized nor normalized.
    path: PathBuf,
}

impl ImportClause {
    pub const PREFIX: &'static str = "#import ";
}

impl<P: Into<PathBuf>> From<P> for ImportClause {
    fn from(path: P) -> Self {
        Self { path: path.into() }
    }
}

impl std::str::FromStr for ImportClause {
    type Err = anyhow::Error;

    fn from_str(clause_str: &str) -> Result<Self, Self::Err> {
        let s = clause_str.trim();

        ensure!(
            s.starts_with(Self::PREFIX),
            "import clause must start with {prefix:?}, got {s:?}",
            prefix = Self::PREFIX,
        );
        let s = s.trim_start_matches(Self::PREFIX).trim();

        let rs = s.chars().rev().collect::<String>();

        let splits = s
            .find('<')
            .and_then(|i0| rs.find('>').map(|i1| (i0 + 1, rs.len() - i1 - 1)));

        if let Some((i0, i1)) = splits {
            let s = &s[i0..i1];
            ensure!(!s.is_empty(), "import clause must contain a non-empty path");

            return s
                .parse()
                .with_context(|| format!("couldn't parse {s:?} as PathBuf"))
                .map(|path| Self { path });
        }

        bail!("malformed import clause: {clause_str:?}")
    }
}

fn check_hermeticity(root_path: impl AsRef<Path>, file_path: impl AsRef<Path>) {
    let file_path = file_path.as_ref();
    let dir_path = file_path.parent().unwrap();
    std::fs::read_to_string(file_path)
        .unwrap()
        .lines()
        .try_for_each(|line| {
            if !line.trim().starts_with(ImportClause::PREFIX) {
                return Ok(());
            }

            let clause = line.parse::<ImportClause>()?;
            let clause_path = dir_path.join(clause.path);
            let clause_path = std::fs::canonicalize(clause_path)?;
            ensure!(
                clause_path.starts_with(&root_path),
                "trying to import {clause_path:?} which lives outside of the workspace, \
                    this is illegal in release and/or Wasm builds!"
            );

            Ok::<_, anyhow::Error>(())
        })
        .unwrap();
}

// ---

fn should_run(environment: Environment) -> bool {
    #![expect(clippy::match_same_arms)]

    match environment {
        // we should have been run before publishing
        Environment::PublishingCrates => false,

        // The code we're generating here is actual source code that gets committed into the repository.
        Environment::RerunCI | Environment::CondaBuild => false,

        Environment::DeveloperInWorkspace => true,

        Environment::UsedAsDependency => false,
    }
}

/// Compute SHA-256 hash of a file's contents
fn compute_file_hash(path: &Path) -> anyhow::Result<String> {
    let content = std::fs::read(path)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;
    let hash = Sha256::digest(&content);
    Ok(format!("{hash:x}"))
}

/// Load the shader manifest from target directory (persistent across incremental builds)
fn load_shader_manifest(manifest_dir: &Path) -> BTreeMap<PathBuf, String> {
    let manifest_path = manifest_dir.join("shader_manifest.json");
    if let Ok(content) = std::fs::read_to_string(&manifest_path) {
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        BTreeMap::new()
    }
}

/// Save the shader manifest to target directory (persistent across incremental builds)
fn save_shader_manifest(manifest_dir: &Path, manifest: &BTreeMap<PathBuf, String>) -> anyhow::Result<()> {
    std::fs::create_dir_all(manifest_dir)?;
    let manifest_path = manifest_dir.join("shader_manifest.json");
    let content = serde_json::to_string_pretty(manifest)?;
    std::fs::write(&manifest_path, content)?;
    Ok(())
}

/// Check if shaders need to be rebuilt by comparing current hashes with cached manifest
fn should_rebuild_shaders(
    entries: &[DirEntry],
    shader_dir: &Path,
    manifest_dir: &Path,
) -> anyhow::Result<bool> {
    let previous_manifest = load_shader_manifest(manifest_dir);

    // Build current manifest
    let mut current_manifest = BTreeMap::new();
    for entry in entries {
        let path = entry.path();
        let relative_path = path.strip_prefix(shader_dir)
            .unwrap_or(path)
            .to_path_buf();
        let hash = compute_file_hash(path)?;
        current_manifest.insert(relative_path, hash);
    }

    // Compare manifests
    if current_manifest != previous_manifest {
        // Save new manifest
        save_shader_manifest(manifest_dir, &current_manifest)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn main() {
    let environment = Environment::detect();
    let is_release = cfg!(not(debug_assertions)); // This works

    // DO NOT USE `cfg!` for this, that would give you the host's platform!
    let targets_wasm = get_and_track_env_var("CARGO_CFG_TARGET_FAMILY").unwrap() == "wasm";

    cfg_aliases::cfg_aliases! {
        native: { not(target_arch = "wasm32") },
        web: { target_arch = "wasm32" },
    }

    println!("cargo::rustc-check-cfg=cfg(load_shaders_from_disk)");
    if environment == Environment::DeveloperInWorkspace && !is_release && !targets_wasm {
        // Enable hot shader reloading:
        println!("cargo:rustc-cfg=load_shaders_from_disk");
    }

    if !should_run(environment) {
        return;
    }

    // Root path of the re_renderer crate.
    //
    // We're packing at that level rather than at the workspace level because we lose all workspace
    // layout information when publishing the crates.
    // This means all the shaders we pack must live under `re_renderer/shader` for now.
    let manifest_path = Path::new(&get_and_track_env_var("CARGO_MANIFEST_DIR").unwrap()).to_owned();
    let shader_dir = manifest_path.join("shader");

    // On windows at least, it's been shown that the paths we get out of these env-vars can
    // actually turn out _not_ to be canonicalized in practice, which of course will break
    // hermeticity checks later down the line.
    //
    // So: canonicalize them all, just in case… ¯\_(ツ)_/¯
    let manifest_path = std::fs::canonicalize(manifest_path).unwrap();
    let shader_dir = std::fs::canonicalize(shader_dir).unwrap();

    let src_path = manifest_path.join("src");
    let file_path = src_path.join("workspace_shaders.rs");

    fn is_wgsl_or_dir(entry: &DirEntry) -> bool {
        let is_dir = entry.file_type().is_dir();
        let is_wgsl = entry
            .file_name()
            .to_str()
            .is_some_and(|s| s.ends_with(".wgsl"));
        is_dir || is_wgsl
    }

    // We do our best to generate code that passes rustfmt, even though we also
    // add `#[rustfmt::skip]` to the whole module.

    let mut contents = r#"// This file is autogenerated via build.rs.
// DO NOT EDIT.

use std::path::Path;

static ONCE: ::std::sync::atomic::AtomicBool = ::std::sync::atomic::AtomicBool::new(false);

pub fn init() {
    if ONCE.swap(true, ::std::sync::atomic::Ordering::Relaxed) {
        return;
    }

    use crate::file_system::FileSystem as _;
    let fs = crate::MemFileSystem::get();
"#
    .to_owned();

    let walker = WalkDir::new(&shader_dir).into_iter();
    let entries = {
        let mut entries = walker
            .filter_entry(is_wgsl_or_dir)
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_file())
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.path().cmp(b.path()));
        entries
    };

    assert!(
        !entries.is_empty(),
        "re_renderer build.rs found no shaders - I think some path is wrong!"
    );

    // Register all shader files with Cargo's change tracking first
    for entry in &entries {
        rerun_if_changed(entry.path());
    }

    // Check if we need to rebuild based on shader content hashes
    // Use target directory for persistent caching across incremental builds
    let target_dir = PathBuf::from(get_and_track_env_var("OUT_DIR").unwrap())
        .ancestors()
        .nth(3) // OUT_DIR is target/<profile>/build/<crate>-<hash>/out, go up to target/
        .unwrap()
        .to_path_buf()
        .join("re_renderer_cache");

    if !should_rebuild_shaders(&entries, &shader_dir, &target_dir).unwrap() {
        println!("cargo:warning=Shaders unchanged, skipping regeneration");
        return;
    }

    println!("cargo:warning=Shader changes detected, regenerating workspace_shaders.rs");

    for entry in entries {

        // The relative path to get from the current shader file to `workspace_shaders.rs`.
        // We must make sure to pass relative paths to `include_str`!
        let relpath = pathdiff::diff_paths(entry.path(), &src_path).unwrap();
        let relpath = relpath.to_str().unwrap().replace('\\', "/"); // Force slashes on Windows.

        // The hermetic path used in the virtual filesystem at run-time.
        //
        // This is using the exact same strip_prefix as the standard `file!()` macro, so that
        // hermetic paths generated by one will be comparable with the hermetic paths generated
        // by the other!
        let virtpath = entry.path().strip_prefix(&manifest_path).unwrap();
        let virtpath = virtpath.to_str().unwrap().replace('\\', "/"); // Force slashes on Windows.

        // Make sure we're not referencing anything outside of the workspace!
        //
        // TODO(cmc): At the moment we only look for breaches of hermiticity at the import level
        // and completely ignore top-level, e.g. `#import </tmp/shader.wgsl>` will fail as
        // expected in release builds, while `include_file!("/tmp/shader.wgsl")` won't!
        //
        // The only way to make hermeticity checks work for top-level files would be to read all
        // Rust files and parse all `include_file!` statements in those, so that we actually
        // know what those external top-level files are to begin with.
        // Not worth it… for now.
        if is_release || targets_wasm {
            check_hermeticity(&manifest_path, entry.path()); // will fail if not hermetic
        }

        contents += &format!(
            "
    {{
        let virtpath = Path::new(\"{virtpath}\");
        let content = include_str!(\"{relpath}\").into();
        fs.create_file(virtpath, content).unwrap();
    }}
",
        );
    }

    contents = format!("{}\n}}\n", contents.trim_end());

    write_file_if_necessary(file_path, contents.as_bytes()).unwrap();
}
