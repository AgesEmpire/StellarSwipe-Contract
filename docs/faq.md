# Soroban development FAQ (StellarSwipe)

Answers here are tailored to **this repository** (`stellar-swipe/`, `soroban-sdk` workspace version, scripts, and patterns). They synthesize recurring Soroban topics and how we apply them here.

**Sources to mine for new entries:** team chat threads, GitHub issues, and PR review comments — when you add a question, link the discussion in your PR description if it is public.

---

## How we keep this FAQ up to date

1. **Any PR** that changes Soroban auth, storage, or test utilities should add or adjust an FAQ entry if reviewers saw repeated confusion.
2. **Quarterly (or on Soroban SDK bumps):** a volunteer runs through the [Review checklist](#faq-review-checklist-for-contributors) and opens a small PR for stale wording or CLI flag drift.
3. **Template for new items:** open a PR that appends one **Question → Answer → Example** block; assign **two reviewers** who touch contracts regularly (see done criteria for your issue tracker).

### FAQ review checklist (for contributors)

- [ ] Code snippets compile against the workspace `soroban-sdk` version in `stellar-swipe/Cargo.toml`.
- [ ] Storage examples name the correct **instance / persistent / temporary** bucket.
- [ ] Auth examples distinguish **on-chain** `require_auth` from **test** `mock_all_auths`.
- [ ] Cross-contract example matches how we call oracles (`invoke_contract`) or clients.
- [ ] Build section matches `stellar-swipe/scripts/build.sh` and documented `wasm32-unknown-unknown` flow.

---

## Build & toolchain

### 1. Why does `cargo build` fail with “can't find crate for `core`” or wrong target errors?

**Answer:** Soroban contracts compile to **WebAssembly**. You must install the `wasm32-unknown-unknown` Rust target for the toolchain you use to build contracts.

**Example:**

```bash
rustup target add wasm32-unknown-unknown
cd stellar-swipe
cargo build --workspace --target wasm32-unknown-unknown --release
```

---

### 2. What is the supported way to produce small `.wasm` artifacts in this repo?

**Answer:** Use the **release** profile in `stellar-swipe/Cargo.toml` (`opt-level = "z"`, `lto`, `strip`, etc.), then run **`stellar contract optimize`** via our script so deploy/upload sizes stay reasonable.

**Example:**

```bash
cd stellar-swipe
./scripts/build.sh
# Optimized artifacts: target/wasm-optimized/*.wasm
```

Requires `stellar` on `PATH` (see comments in `stellar-swipe/scripts/build.sh`).

---

### 3. The build script says `stellar CLI not found` — what should I install?

**Answer:** The script shells out to **`stellar contract optimize`**. Install the Stellar CLI (formerly some workflows used `soroban` only); follow current Stellar docs for your OS.

**Example:**

```bash
cargo install stellar-cli --locked
command -v stellar
```

---

### 4. Why do I get a huge `*.wasm` if I only run `cargo build --release`?

**Answer:** Rust’s release Wasm is still larger than **wasm-opt** output. CI and deploys in this project expect the **optimized** artifact when comparing sizes or uploading.

**Example:**

```bash
cd stellar-swipe
./scripts/build.sh --compare   # optional: debug vs release vs optimized table
```

---

### 5. `cargo build --workspace` fails in a crate that is not a contract — is that expected?

**Answer:** The workspace under `stellar-swipe/` is contract-centric. Build with `--target wasm32-unknown-unknown` only for contract crates; library crates like `common` compile as dependencies. If a binary crate slipped into `members`, remove it or gate it behind non-Wasm targets.

**Example:**

```bash
cd stellar-swipe
cargo check -p stellar-swipe-common
cargo build -p stellar-swipe-signal-registry --target wasm32-unknown-unknown --release
```

(Replace `-p` with the exact package name from the crate’s `Cargo.toml`.)

---

## Auth & custom account rules

### 6. My contract entrypoint returns an auth error in tests — what is the first thing to check?

**Answer:** In unit tests, nothing signs unless you **mock** authorizations. Our tests commonly call `env.mock_all_auths()` once per test so `Address::require_auth()` paths succeed. On-chain, the **same** entrypoint requires a real signature from the declared address.

**Example:**

```rust
use soroban_sdk::{testutils::Address as _, Env};

#[test]
fn admin_init() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    // client.initialize(&admin) will record admin.require_auth() as satisfied in tests
}
```

---

### 7. What is the difference between `client.foo` and `client.try_foo` in tests?

**Answer:** `try_*` returns `Result` so you can `assert!(result.is_err())` without panicking. Use it for **negative** tests (double init, invalid args).

**Example:**

```rust
let result = client.try_initialize(&other_admin);
assert!(result.is_err());
```

---

### 8. Who must `require_auth` — the caller argument or stored admin?

**Answer:** The **address that must authorize this invocation** must call `require_auth()` on itself (typically the first step after loading state). If you `require_auth` for `admin` but the transaction is signed by a user, the host rejects it. Align tests: generate an `Address` and pass it as the same role the contract checks.

**Example:**

```rust
pub fn pause(env: Env, caller: Address) {
    caller.require_auth();
    // then assert caller == stored_admin or guardian policy
}
```

---

## Storage & TTL

### 9. When should I use `instance`, `persistent`, or `temporary` storage?

**Answer:**

- **Instance:** Small, hot config tied to the contract instance (often one map); lives with the instance TTL model you extend at the contract level.
- **Persistent:** User balances, history, anything that must **survive** and be explicitly TTL-managed per key.
- **Temporary:** Ephemeral analytics or mocks — can expire quickly; good for **test doubles** and rolling buckets.

**Example (temporary mock in `common`):**

```rust
env.storage()
    .temporary()
    .set(&(symbol_short!("mock_orc"), asset_pair), &price);
```

---

### 10. Why does my persistent map “disappear” or return `None` after many ledgers?

**Answer:** **TTL** expired. Persistent entries need **`extend_ttl`** (or bump on every write, depending on policy) or they become inaccessible when their live_until_ledger passes.

**Example (oracle history pattern in this repo):**

```rust
const DAY_IN_LEDGERS: u32 = 17280;

env.storage().persistent().set(&key, &price);
env.storage()
    .persistent()
    .extend_ttl(&key, DAY_IN_LEDGERS * 7, DAY_IN_LEDGERS * 7);
```

---

### 11. I write to `temporary` every ledger — do I still need `extend_ttl`?

**Answer:** Yes, if you need the entry to live **longer than the default temporary TTL**. Fee analytics, for example, sets the bucket then extends.

**Example:**

```rust
let t = env.storage().temporary();
t.set(&key, &bucket);
t.extend_ttl(
    &key,
    TEMP_FEE_BUCKET_TTL_LEDGERS,
    TEMP_FEE_BUCKET_TTL_LEDGERS,
);
```

---

### 12. Can I use `std::collections::HashMap` in contract code?

**Answer:** No. Contracts are `#![no_std]`. Use Soroban **`Map`**, **`Vec`**, and types marked **`#[contracttype]`** so the SDK can serialize them to storage and events.

**Example:**

```rust
use soroban_sdk::{Env, Map};

let mut m: Map<u32, u64> = Map::new(env);
m.set(1, 42);
env.storage().instance().set(&StorageKey::MyMap, &m);
```

---

## Cross-contract calls

### 13. How do I call another contract’s function from this codebase’s style?

**Answer:** Use `Env::invoke_contract` with a **`Symbol`** for the function name and a Soroban **`Vec`** of arguments. The oracle wrapper uses this pattern.

**Example (from `stellar-swipe/contracts/common/src/oracle.rs`):**

```rust
use soroban_sdk::{symbol_short, vec, Address, Env, Symbol};

let result: Option<OraclePrice> = env.invoke_contract(
    &oracle_address,
    &Symbol::new(env, "get_price"),
    vec![env, asset_pair.into()],
);
```

---

### 14. How do I avoid duplicating client logic for tests vs production (oracle)?

**Answer:** Define a **trait** (`IOracleClient`) with two implementations: `OnChainOracleClient` (invoke) and `MockOracleClient` (temporary storage). Swap implementations in tests without changing business logic.

**Example:**

```rust
pub trait IOracleClient {
    fn get_price(&self, env: &Env, asset_pair: u32) -> Result<OraclePrice, OracleError>;
}
```

---

### 15. My cross-contract call fails with a trapped error — how do I debug?

**Answer:** Confirm (1) callee contract ID, (2) **function symbol** spelling, (3) argument order/types in `vec![env, …]`, (4) callee **auth** requirements (nested calls still need appropriate auth contexts). In tests, register the callee first and use `try_*` on the callee to isolate failures.

**Example:**

```rust
let callee_id = env.register(CalleeContract, ());
let _: u64 = env.invoke_contract(
    &callee_id,
    &Symbol::new(env, "get_count"),
    vec![env],
);
```

---

## Testing patterns

### 16. How do I register a contract in tests in this repo?

**Answer:** Older tests use `env.register_contract(None, MyContract)`; newer SDK patterns may use `env.register`. Follow existing tests in the crate you edit for consistency.

**Example:**

```rust
#[allow(deprecated)]
let contract_id = env.register_contract(None, SignalRegistry);
let client = SignalRegistryClient::new(&env, &contract_id);
```

---

### 17. How do I mint test tokens for a user in executor-style tests?

**Answer:** Register a Stellar asset contract, then use **`StellarAssetClient::mint`** (with auths mocked).

**Example:**

```rust
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::testutils::Address as _;

let env = Env::default();
env.mock_all_auths();
let issuer = Address::generate(&env);
let sac = env.register_stellar_asset_contract_v2(issuer);
let token = sac.address();
let user = Address::generate(&env);
StellarAssetClient::new(&env, &token).mint(&user, &1_000_000i128);
```

---

### 18. Why does `String::from_str` fail or panic in tests?

**Answer:** Soroban `String` is bounded and host-managed. Use `String::from_str(env, "literal")` and keep strings within contract limits; for dynamic data prefer validated inputs and early `Err` returns.

**Example:**

```rust
let label = String::from_str(&env, "XLM/USDC");
client.create_signal(&provider, &label, /* … */);
```

---

### 19. What is the right way to build argument vectors for clients / invoke?

**Answer:** Use Soroban’s **`vec!` macro** with `&env` as the first parameter for `soroban_sdk::Vec`.

**Example:**

```rust
use soroban_sdk::vec;

let tags = vec![&env, String::from_str(&env, "test")];
```

---

### 20. How do I assert on contract errors cleanly?

**Answer:** Use **`try_*`** client methods and match on the error variant, or map contract-specific `Result` types in pure Rust tests (no host).

**Example:**

```rust
let result = client.try_create_signal(/* … */);
assert!(result.is_err());
```

---

## Types, limits, and misc

### 21. Why must my custom storage keys derive `Clone` and use `#[contracttype]`?

**Answer:** Keys and values stored in contract storage must use SDK-compatible types the host can serialize. `#[contracttype]` generates the needed glue; keys are often enums.

**Example:**

```rust
#[contracttype]
#[derive(Clone)]
pub enum StorageKey {
    Admin,
    Signals,
}
```

---

### 22. I hit WASM size limits or instruction metering — what knobs exist?

**Answer:** Shrink code: higher optimization (`opt-level = "z"`), `lto`, strip symbols, split logic into smaller modules, avoid monolithic `match` trees, and run **`stellar contract optimize`**. Also reduce **debug strings** and heavy `String` formatting in hot paths.

**Example:**

```toml
# stellar-swipe/Cargo.toml — already configured for contracts
[profile.release]
opt-level = "z"
lto = true
strip = "symbols"
codegen-units = 1
```

---

### 23. `symbol_short!` vs `Symbol::new` — which should I use?

**Answer:** `symbol_short!` only accepts **≤9 characters** and is compile-time friendly. Longer method names need `Symbol::new(env, "long_name")`.

**Example:**

```rust
use soroban_sdk::{symbol_short, Symbol};

let a = symbol_short!("get_price");
let b = Symbol::new(env, "get_open_position_count");
```

---

### 24. Where is the Soroban SDK version pinned for this workspace?

**Answer:** In `stellar-swipe/Cargo.toml` under `[workspace.dependencies]`. FAQ examples must match that major/minor (`soroban-sdk = "23"` at time of writing).

**Example:**

```toml
[workspace.dependencies]
soroban-sdk = "23"
```

---

## FAQ review checklist for contributors

Use this when you **edit** this file or when doing a periodic accuracy pass:

| Step | Action |
|------|--------|
| 1 | Confirm commands (`stellar`, `cargo` targets) match current team setup. |
| 2 | Re-run at least one workspace `cargo test -p …` for a crate touched by new examples. |
| 3 | Cross-check storage guidance against real usage (`instance` vs `persistent` vs `temporary`). |
| 4 | If SDK was upgraded, grep this FAQ for deprecated APIs and fix in the same bump PR. |
| 5 | Add a **Change log** table row below when guidance materially shifts. |

| Date | Change |
|------|--------|
| 2026-04-22 | Initial FAQ and maintenance process added. |

---

*If two contributors have **not** yet signed off on accuracy for your PR, keep the PR in draft or block merge per your team’s done criteria.*
