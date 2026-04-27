import WalletButton from "@/components/WalletButton";

export default function Home() {
  return (
    <>
      <nav
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
          padding: "16px 24px",
          borderBottom: "1px solid #2a2a4a",
        }}
      >
        <span style={{ fontWeight: 700, fontSize: 20, letterSpacing: 1 }}>
          ✦ StellarSwipe
        </span>
        <WalletButton />
      </nav>

      <main style={{ padding: "48px 24px", textAlign: "center" }}>
        <h1 style={{ fontSize: 36, marginBottom: 12 }}>Swipe. Copy. Trade.</h1>
        <p style={{ color: "#9090c0" }}>
          Connect your Freighter wallet to start copy-trading on Stellar.
        </p>
      </main>
    </>
  );
}
