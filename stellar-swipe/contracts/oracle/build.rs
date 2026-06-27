fn main() {
    // Propagate SOURCE_HASH from the environment (set by scripts/build.sh) so
    // contractmeta!(key="source_hash", ...) can embed it at compile time.
    // Falls back to "dev-build" when building locally without build.sh.
    let hash = std::env::var("SOURCE_HASH").unwrap_or_else(|_| "dev-build".to_string());
    println!("cargo:rustc-env=SOURCE_HASH={hash}");
    println!("cargo:rerun-if-env-changed=SOURCE_HASH");
}
