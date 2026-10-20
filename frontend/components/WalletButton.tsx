"use client";
import { useEffect, useRef, useState } from "react";
import { useWallet } from "@/hooks/useWallet";

function truncate(addr: string) {
  return `${addr.slice(0, 6)}…${addr.slice(-4)}`;
}

export default function WalletButton() {
  const { address, connecting, connect, disconnect } = useWallet();
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    function handler(e: MouseEvent) {
      if (!containerRef.current?.contains(e.target as Node)) setOpen(false);
    }
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  // Close on Escape
  useEffect(() => {
    if (!open) return;
    function handler(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [open]);

  async function copyAddress() {
    if (!address) return;
    await navigator.clipboard.writeText(address);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }

  if (!address) {
    return (
      <button
        onClick={connect}
        disabled={connecting}
        style={styles.btn}
        aria-label="Connect wallet"
      >
        {connecting ? "Connecting…" : "Connect Wallet"}
      </button>
    );
  }

  return (
    <div ref={containerRef} style={styles.container}>
      <button
        onClick={() => setOpen((o) => !o)}
        style={styles.btn}
        aria-haspopup="true"
        aria-expanded={open}
        aria-label="Wallet options"
      >
        {truncate(address)}
        <span style={styles.chevron}>{open ? "▲" : "▼"}</span>
      </button>

      {open && (
        <div role="menu" style={styles.dropdown}>
          <p style={styles.fullAddress} title={address}>
            {address}
          </p>
          <hr style={styles.divider} />
          <button
            role="menuitem"
            onClick={copyAddress}
            style={styles.menuItem}
          >
            {copied ? "✓ Copied!" : "Copy Address"}
          </button>
          <button
            role="menuitem"
            onClick={() => { disconnect(); setOpen(false); }}
            style={{ ...styles.menuItem, ...styles.disconnect }}
          >
            Disconnect
          </button>
        </div>
      )}
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  container: { position: "relative", display: "inline-block" },
  btn: {
    display: "flex",
    alignItems: "center",
    gap: 6,
    padding: "8px 16px",
    background: "#1a1a2e",
    color: "#e0e0ff",
    border: "1px solid #4a4a8a",
    borderRadius: 8,
    cursor: "pointer",
    fontSize: 14,
    fontWeight: 600,
    whiteSpace: "nowrap",
  },
  chevron: { fontSize: 10, opacity: 0.7 },
  dropdown: {
    position: "absolute",
    right: 0,
    top: "calc(100% + 6px)",
    minWidth: 280,
    background: "#1a1a2e",
    border: "1px solid #4a4a8a",
    borderRadius: 8,
    padding: "8px 0",
    zIndex: 100,
    boxShadow: "0 8px 24px rgba(0,0,0,0.4)",
  },
  fullAddress: {
    margin: "4px 16px 8px",
    fontSize: 11,
    color: "#9090c0",
    wordBreak: "break-all",
    fontFamily: "monospace",
  },
  divider: { border: "none", borderTop: "1px solid #2a2a4a", margin: "4px 0" },
  menuItem: {
    display: "block",
    width: "100%",
    padding: "10px 16px",
    background: "none",
    border: "none",
    color: "#e0e0ff",
    textAlign: "left",
    cursor: "pointer",
    fontSize: 14,
  },
  disconnect: { color: "#ff6b6b" },
};
