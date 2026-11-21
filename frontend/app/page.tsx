import WalletButton from "@/components/WalletButton";
import { getNetworkMetadata } from "@/src/config/rpc-endpoints";

export default function Home() {
  const network = getNetworkMetadata();

  return (
    <main className="shell">
      <section className="hero">
        <div className="brand-row">
          <span className="brand-mark">✦</span>
          <span className="brand-name">StellarSwipe</span>
        </div>

        <div className="hero-copy">
          <p className="eyebrow">Network-aware contract dashboard</p>
          <h1>Swipe. Copy. Trade.</h1>
          <p className="lede">
            The frontend now resolves the active Stellar network from{" "}
            <code>NEXT_PUBLIC_STELLAR_NETWORK</code> and uses the matching RPC,
            Horizon, and network passphrase from the shared config file.
          </p>
        </div>

        <div className="network-card">
          <div>
            <span className="label">Selected network</span>
            <strong>{network.network}</strong>
          </div>
          <div>
            <span className="label">RPC</span>
            <code>{network.rpc}</code>
          </div>
          <div>
            <span className="label">Fallback RPC</span>
            <code>{network.fallbackRpc}</code>
          </div>
          <div>
            <span className="label">Horizon</span>
            <code>{network.horizonUrl}</code>
          </div>
        </div>

        <div className="cta-row">
          <WalletButton />
          <a
            className="secondary-link"
            href={network.horizonUrl}
            target="_blank"
            rel="noreferrer"
          >
            Open Horizon
          </a>
        </div>
      </section>
    </main>
  );
}
