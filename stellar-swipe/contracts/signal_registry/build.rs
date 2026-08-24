fn main() {
    let hash = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            std::env::var("GIT_COMMIT_HASH").unwrap_or_else(|_| "unknown".to_string())
        });

    if hash == "unknown" && std::env::var("CI").is_ok() {
        panic!("GIT_COMMIT_HASH could not be determined in CI — ensure git history is available");
    }

    println!("cargo:rustc-env=GIT_COMMIT_HASH={hash}");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-env-changed=GIT_COMMIT_HASH");
}
