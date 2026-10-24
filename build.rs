use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    println!("cargo:rerun-if-env-changed=NEKO_SOURCE_REVISION");
    // Re-evaluate for source changes, including an uncommitted worktree build.
    for path in ["src", "crates", "Cargo.toml", "Cargo.lock", "config.example.json", "build.rs", "scripts", "docs", ".gitignore"] {
        println!("cargo:rerun-if-changed={path}");
    }
    for name in ["HEAD", "index", "packed-refs"] {
        if let Some(path) = git(&["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git(&["rev-parse", "--git-path", &reference]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    let supplied = std::env::var("NEKO_SOURCE_REVISION").ok().filter(|s| !s.is_empty() && s.len() <= 80 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
    let mut revision = supplied.unwrap_or_else(|| git(&["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "source-archive".into()));
    if git(&["status", "--porcelain", "--untracked-files=normal"]).is_some_and(|s| !s.is_empty()) {
        revision.push_str("-dirty");
    }
    println!("cargo:rustc-env=NEKO_BUILD_REVISION={revision}");
}
