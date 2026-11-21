# Release fixture results

Results from `contracts/integration_tests/tests/integration/test_release_fixture.rs`
(Issue #1341), recorded by the "Run release fixture against optimized WASM"
step in `.github/workflows/ci.yml`.

`latest.json` records, for the most recent CI run on `main`:

- `artifact`: the optimized WASM file exercised (currently `signal_registry.wasm`)
- `sha256`: its content hash, extracted from the `RELEASE_FIXTURE_ARTIFACT_HASH`
  line the test prints to stderr
- `result`: `pass` or `fail`
- `commit`: the git SHA the artifact was built from
- `recorded_at`: UTC timestamp of the CI run

This gives a reproducible, auditable link between "this exact WASM hash" and
"these entrypoints behaved correctly when deployed," independent of any
native-test-only pass.
