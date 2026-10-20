#!/usr/bin/env python3
import os
import subprocess
import sys

def check_no_std_attribute(crate_path):
    lib_rs_path = os.path.join(crate_path, "src", "lib.rs")
    if not os.path.exists(lib_rs_path):
        print(f"Error: src/lib.rs not found in {crate_path}", file=sys.stderr)
        return False

    with open(lib_rs_path, "r", encoding="utf-8") as f:
        content = f.read()

    # Clean comments to avoid false matches
    lines = content.splitlines()
    cleaned_lines = []
    for line in lines:
        stripped = line.strip()
        if not stripped.startswith("//") and not stripped.startswith("/*"):
            cleaned_lines.append(stripped)
    cleaned_content = "\n".join(cleaned_lines)

    if "#![no_std]" not in cleaned_content:
        print(f"Violation: '#![no_std]' attribute is missing in {lib_rs_path}", file=sys.stderr)
        return False
    return True

def main():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    workspace_dir = os.path.dirname(script_dir)

    contract_crates = [
        "signal_registry",
        "auto_trade",
        "trade_executor",
        "governance",
        "oracle",
        "stake_vault",
        "fee_collector",
        "user_portfolio",
        "analytics",
        "bridge"
    ]

    failed = False

    print("Checking '#![no_std]' attribute in contract crates...")
    for crate in contract_crates:
        crate_path = os.path.join(workspace_dir, "contracts", crate)
        if not check_no_std_attribute(crate_path):
            print(f"-> Crate '{crate}' failed syntactic no_std check.", file=sys.stderr)
            failed = True
        else:
            print(f"-> Crate '{crate}' passed syntactic no_std check.")

    print("\nVerifying WASM build compatibility (no-std build target)...")
    for crate in contract_crates:
        print(f"Compiling crate '{crate}' to wasm32-unknown-unknown...")
        try:
            # We run cargo build with target wasm32-unknown-unknown
            result = subprocess.run(
                ["cargo", "build", "--package", crate, "--target", "wasm32-unknown-unknown", "--release"],
                cwd=workspace_dir,
                capture_output=True,
                text=True,
                check=True
            )
            print(f"-> Crate '{crate}' built successfully for WASM target.")
        except subprocess.CalledProcessError as e:
            print(f"Violation: Crate '{crate}' failed to build for WASM target.", file=sys.stderr)
            print(f"Cargo Error Output:\n{e.stderr}", file=sys.stderr)
            failed = True

    if failed:
        print("\nVerification FAILED: One or more contract crates are not #![no_std] compliant.", file=sys.stderr)
        sys.exit(1)
    else:
        print("\nVerification PASSED: All contract crates are #![no_std] compliant.")
        sys.exit(0)

if __name__ == "__main__":
    main()
