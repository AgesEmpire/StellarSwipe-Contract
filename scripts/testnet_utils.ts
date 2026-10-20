#!/usr/bin/env tsx
/**
 * testnet_utils.ts — Testnet utility helpers for StellarSwipe.
 *
 * Usage:
 *   npx tsx scripts/testnet_utils.ts fund <G...address>
 *
 * Functions:
 *   fundTestnetAccount(address) — calls Stellar Friendbot to fund a new testnet account.
 *
 * Constraint: this is a script utility only, not a contract function.
 * Mainnet guard: throws immediately if STELLAR_NETWORK is not "testnet".
 */

// ── Mainnet guard ─────────────────────────────────────────────────────────────

const NETWORK = process.env.STELLAR_NETWORK ?? "testnet";

if (NETWORK !== "testnet") {
  console.error(
    `ERROR: testnet_utils.ts must only be used on testnet. ` +
    `Current STELLAR_NETWORK="${NETWORK}". Aborting.`
  );
  process.exit(1);
}

// ── Constants ─────────────────────────────────────────────────────────────────

const FRIENDBOT_URL = "https://friendbot.stellar.org";

// ── Types ─────────────────────────────────────────────────────────────────────

interface FundResult {
  address: string;
  funded: boolean;
  /** Friendbot transaction hash on success */
  txHash?: string;
  /** Human-readable error message on failure */
  error?: string;
}

// ── Core function ─────────────────────────────────────────────────────────────

/**
 * Fund a new testnet account via Stellar Friendbot.
 *
 * Only available on testnet — throws if STELLAR_NETWORK !== "testnet".
 *
 * @param address - Stellar public key (G...) to fund
 * @returns FundResult with funded status and transaction hash
 */
export async function fundTestnetAccount(address: string): Promise<FundResult> {
  // Re-check guard in case the function is imported and called directly
  if ((process.env.STELLAR_NETWORK ?? "testnet") !== "testnet") {
    throw new Error(
      "fundTestnetAccount() is only available on testnet. " +
      `STELLAR_NETWORK="${process.env.STELLAR_NETWORK}"`
    );
  }

  if (!address || !address.startsWith("G") || address.length !== 56) {
    return {
      address,
      funded: false,
      error: `Invalid Stellar address: "${address}". Must be a 56-character G... public key.`,
    };
  }

  const url = `${FRIENDBOT_URL}?addr=${encodeURIComponent(address)}`;
  console.log(`[fund] Requesting Friendbot for ${address} ...`);

  let response: Response;
  try {
    response = await fetch(url);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    return { address, funded: false, error: `Network error calling Friendbot: ${msg}` };
  }

  // Friendbot returns 200 on success, 400 if already funded
  const body = await response.text();

  if (response.ok) {
    let txHash: string | undefined;
    try {
      const json = JSON.parse(body) as { hash?: string; id?: string };
      txHash = json.hash ?? json.id;
    } catch {
      // body is not JSON — that's fine, hash is optional
    }
    console.log(`[fund] Success — account ${address} funded.${txHash ? ` tx: ${txHash}` : ""}`);
    return { address, funded: true, txHash };
  }

  // Parse Friendbot error detail
  let detail = body;
  try {
    const json = JSON.parse(body) as { detail?: string; title?: string };
    detail = json.detail ?? json.title ?? body;
  } catch {
    // use raw body
  }

  // 400 "createAccountAlreadyExist" is not a fatal error — account is already funded
  if (detail.toLowerCase().includes("already") || detail.toLowerCase().includes("exist")) {
    console.log(`[fund] Account ${address} is already funded on testnet.`);
    return { address, funded: true, error: "Account already funded (Friendbot 400)" };
  }

  return {
    address,
    funded: false,
    error: `Friendbot returned HTTP ${response.status}: ${detail}`,
  };
}

// ── CLI entrypoint ────────────────────────────────────────────────────────────

async function main(): Promise<void> {
  const [, , command, address] = process.argv;

  if (command === "fund") {
    if (!address) {
      console.error("Usage: npx tsx scripts/testnet_utils.ts fund <G...address>");
      process.exit(1);
    }

    const result = await fundTestnetAccount(address);

    if (!result.funded) {
      console.error(`[fund] FAILED: ${result.error}`);
      process.exit(1);
    }

    console.log("[fund] PASS — account is funded and ready for testnet use.");
    process.exit(0);
  }

  console.error(`Unknown command: "${command ?? ""}"`);
  console.error("Available commands: fund <G...address>");
  process.exit(1);
}

// Run only when executed directly (not when imported as a module)
if (process.argv[1]?.endsWith("testnet_utils.ts") || process.argv[1]?.endsWith("testnet_utils.js")) {
  main().catch((err) => {
    console.error("[fatal]", err instanceof Error ? err.message : err);
    process.exit(1);
  });
}
