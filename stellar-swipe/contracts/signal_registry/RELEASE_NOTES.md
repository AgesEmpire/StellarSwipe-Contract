# Signal registry release notes

## Breaking changes

- **Versioning API rename:** The historical Soroban entrypoint that performed multi-version signal updates was renamed from `update_signal` to `update_signal_versioned` to avoid a name clash with the new Issue #168 `update_signal` (short post-submit edit of price / rationale hash / confidence). Integrations that called the old versioning method must call `update_signal_versioned` instead.

