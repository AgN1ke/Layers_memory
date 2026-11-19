use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const BUILD_COMMIT: &str = "MEMORY_ENGINE_BUILD_COMMIT";
const BUILD_DIRTY: &str = "MEMORY_ENGINE_BUILD_DIRTY";

fn main() {
    println!("cargo:rerun-if-env-changed={BUILD_COMMIT}");
    println!("cargo:rerun-if-env-changed={BUILD_DIRTY}");
    println!("cargo:rerun-if-env-changed=PATH");
    println!("cargo:rerun-if-changed=build.rs");

    let explicit_commit = build_input(BUILD_COMMIT);
    let explicit_dirty = build_input(BUILD_DIRTY);
    let (commit, dirty) = if explicit_commit.is_some() || explicit_dirty.is_some() {
        let commit = explicit_commit.map(|value| {
            assert!(
                is_full_hash(&value),
                "{BUILD_COMMIT} must be a full Git SHA"
            );
            value.to_ascii_lowercase()
        });
        if let Some(value) = &explicit_dirty {
            assert!(
                matches!(value.as_str(), "true" | "false"),
                "{BUILD_DIRTY} must be true or false"
            );
        }
        (commit, explicit_dirty)
    } else {
        let crate_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
        checkout_identity(&crate_dir.join("../.."))
    };

    println!(
        "cargo:rustc-env=MEMORY_ENGINE_SOURCE_COMMIT={}",
        commit.as_deref().unwrap_or_default()
    );
    println!(
        "cargo:rustc-env=MEMORY_ENGINE_SOURCE_DIRTY={}",
        dirty.as_deref().unwrap_or_default()
    );
}

fn build_input(name: &str) -> Option<String> {
    env::var_os(name).map(|value| {
        value
            .into_string()
            .unwrap_or_else(|_| panic!("{name} must be valid UTF-8"))
    })
}

fn checkout_identity(root: &Path) -> (Option<String>, Option<String>) {
    let marker = root.join(".git");
    if !marker.exists()
        || !git(root, &["rev-parse", "--show-prefix"])
            .is_some_and(|prefix| prefix.trim_end_matches(['\r', '\n']).is_empty())
    {
        return (None, None);
    }
    if marker.is_file() {
        watch(&marker);
    }

    for file in ["HEAD", "index", "logs/HEAD", "packed-refs"] {
        watch_git_file(root, file);
    }
    if let Some(reference) = git(root, &["symbolic-ref", "--quiet", "HEAD"]) {
        watch_git_file(root, reference.trim());
    }
    if let Some(files) = git(root, &["ls-files", "--cached", "-z"]) {
        for file in files.split_terminator('\0') {
            watch(&root.join(file));
        }
    }

    let commit = git(root, &["rev-parse", "--verify", "HEAD^{commit}"])
        .map(|value| value.trim().to_owned())
        .filter(|value| is_full_hash(value));
    let dirty = git(root, &["status", "--porcelain=v1", "--untracked-files=no"])
        .map(|status| (!status.is_empty()).to_string());
    (commit, dirty)
}

fn watch_git_file(root: &Path, file: &str) {
    if let Some(path) = git(
        root,
        &["rev-parse", "--path-format=absolute", "--git-path", file],
    ) {
        let path = Path::new(path.trim_end());
        if path.exists() {
            watch(path);
        } else if file.starts_with("refs/") {
            // A packed ref can gain a loose file on the next commit.
            if let Some(parent) = path.ancestors().skip(1).find(|path| path.is_dir()) {
                watch(parent);
            }
        }
    }
}

fn watch(path: &Path) {
    let path = path.to_string_lossy();
    if !path.contains(['\r', '\n']) {
        println!("cargo:rerun-if-changed={path}");
    }
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("--no-optional-locks")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn is_full_hash(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
