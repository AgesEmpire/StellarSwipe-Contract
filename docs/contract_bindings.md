# Contract Bindings

Frontend clients should use generated TypeScript bindings instead of hand-written
contract call wrappers. The bindings are generated from the compiled Soroban WASM
interface, so function names, parameter types, and return types stay aligned with
the Rust contracts.

## Generate Locally

```bash
cd stellar-swipe
./scripts/build.sh
./scripts/generate_abi.sh
```

By default, the generator reads optimized WASM artifacts from
`target/wasm-optimized` and writes one package per contract under `bindings/`.
Helper WASM files without a Soroban contract interface are skipped.
Each package is created with:

```bash
stellar contract bindings typescript \
  --wasm <contract.wasm> \
  --output-dir bindings/<contract> \
  --overwrite
```

The script also writes `abi.json` beside each generated package using
`stellar contract info interface --output json-formatted`.

## Options

```bash
./scripts/generate_abi.sh --wasm-dir target/wasm32-unknown-unknown/release
./scripts/generate_abi.sh --output-dir ../frontend/src/contracts
./scripts/generate_abi.sh --no-abi-json
```

CI runs the generator after the optimized WASM build and uploads the complete
`bindings/` directory as a build artifact.
