//! The commit the binary is built from, for `dedupe --version` and the help
//! page: `git rev-parse --short=7 HEAD` in the source tree, with `+` when
//! files tracked there differ from it. Built outside a clone (a source
//! archive), the version stands alone.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    // a new commit moves HEAD or the branch it names; an edit changes the
    // index's view of the tree
    for p in [".git/HEAD", ".git/index"] {
        println!("cargo:rerun-if-changed={p}");
    }
    if let Some(head) = git(&["symbolic-ref", "-q", "HEAD"]) {
        println!("cargo:rerun-if-changed=.git/{head}");
    }
    if let Some(sha) = git(&["rev-parse", "--short=7", "HEAD"]).filter(|s| !s.is_empty()) {
        // only source changes count: the docs and launchers do not change
        // the binary
        let dirty = git(&[
            "status",
            "--porcelain",
            "--untracked-files=no",
            "--",
            "src",
            "Cargo.toml",
            "Cargo.lock",
            "build.rs",
        ])
        .is_some_and(|s| !s.is_empty());
        let mark = if dirty { "+" } else { "" };
        println!("cargo:rustc-env=DEDUPE_COMMIT={sha}{mark}");
    }
}
