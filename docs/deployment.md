# Deployment

This document covers how contracts are deployed and how operators verify that a
candidate release is upgrade-compatible with what is already deployed.

## Compatibility manifests

Every contract release publishes a machine-readable **compatibility manifest**.
The manifest is generated as part of the release workflow and is committed
alongside the release artifacts, so operators can diff a deployed version against
a candidate before upgrading.

Each manifest records:

- **Storage layout** — the ordered set of storage slots and their types, used to
  detect storage collisions or reordering between versions.
- **Interface version** — the semantic version of the contract's external
  interface (entry points and their signatures).
- **Upgrade prerequisites** — the conditions that must hold before the upgrade
  can be applied (for example, a minimum source version, required migrations, or
  a paused state).

### Manifest location and format

Manifests are emitted per contract release as JSON, for example:

```json
{
  "contract": "<contract-name>",
  "version": "<release-version>",
  "interfaceVersion": "<semver>",
  "storageLayout": [
    { "slot": 0, "name": "<field>", "type": "<type>" }
  ],
  "upgradePrerequisites": [
    "<prerequisite description>"
  ]
}
```

The release workflow regenerates the manifest for the version being released and
fails the build if the manifest is missing or does not match the compiled
contract.

## CI enforcement

CI rejects any release that:

- does not include a manifest for the released contract, or
- ships a manifest whose storage layout or interface version is incompatible
  with the previously published manifest.

This guarantees that a deployed contract can never be upgraded to a version
whose compatibility metadata was not reviewed.

## Comparing deployed and candidate versions

Operators compare the manifest of the currently deployed version with the
manifest of the candidate release before applying an upgrade:

1. Fetch the manifest for the deployed version from the release artifacts.
2. Fetch the manifest for the candidate version.
3. Diff the two manifests:
   - **Storage layout** — any change in slot ordering or type is a breaking
     change and must be handled by a migration.
   - **Interface version** — a major bump signals breaking interface changes;
     confirm callers are updated.
   - **Upgrade prerequisites** — verify every listed prerequisite is satisfied
     for the deployed instance.
4. Only proceed with the upgrade when the diff is understood and all
   prerequisites are met.

A manifest that is missing, or whose diff shows an unhandled incompatibility,
means the upgrade must not be applied.
