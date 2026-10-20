#!/usr/bin/env python3
import json
import subprocess
import sys
import os

def main():
    # Path to stellar-swipe Cargo.toml
    script_dir = os.path.dirname(os.path.abspath(__file__))
    workspace_dir = os.path.dirname(script_dir)
    manifest_path = os.path.join(workspace_dir, "Cargo.toml")

    # Run cargo metadata
    try:
        result = subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--manifest-path", manifest_path],
            capture_output=True,
            text=True,
            check=True
        )
    except subprocess.CalledProcessError as e:
        print(f"Error running cargo metadata: {e.stderr}", file=sys.stderr)
        sys.exit(1)

    metadata = json.loads(result.stdout)

    # Build a lookup map of package ID -> (name, version)
    pkg_lookup = {}
    for pkg in metadata.get("packages", []):
        pkg_lookup[pkg["id"]] = {
            "name": pkg["name"],
            "version": pkg["version"]
        }

    # Find workspace members
    workspace_members = metadata.get("workspace_members", [])

    # Find the resolve nodes
    resolve = metadata.get("resolve", {})
    nodes = resolve.get("nodes", [])
    node_map = {node["id"]: node for node in nodes}

    # Dependencies we want to audit
    audit_deps = ["soroban-sdk", "stellar-swipe-common", "stellar_swipe_common", "shared"]

    # Map of dep_name -> { member_name: version }
    resolved_versions = {dep: {} for dep in audit_deps}

    for member_id in workspace_members:
        member_pkg = pkg_lookup.get(member_id)
        if not member_pkg:
            continue
        member_name = member_pkg["name"]

        node = node_map.get(member_id)
        if not node:
            continue

        # Look at resolved dependencies of this workspace member
        for dep in node.get("deps", []):
            dep_id = dep.get("pkg")
            dep_pkg = pkg_lookup.get(dep_id)
            if not dep_pkg:
                continue

            dep_name = dep_pkg["name"]
            dep_version = dep_pkg["version"]

            if dep_name in audit_deps:
                resolved_versions[dep_name][member_name] = dep_version

    # Check for mismatches
    has_mismatch = False
    for dep_name, member_map in resolved_versions.items():
        if not member_map:
            continue

        # Get all unique versions
        unique_versions = set(member_map.values())
        if len(unique_versions) > 1:
            print(f"Mismatch found for shared dependency '{dep_name}':", file=sys.stderr)
            for member_name, version in member_map.items():
                print(f"  Crate '{member_name}' resolves to version '{version}'", file=sys.stderr)
            has_mismatch = True

    if has_mismatch:
        print("Audit FAILED: Workspace dependencies are not pinned to the same version.", file=sys.stderr)
        sys.exit(1)
    else:
        print("Audit PASSED: All shared dependencies resolve to matching versions across workspace crates.")
        sys.exit(0)

if __name__ == "__main__":
    main()
