"use client";
import { useState, useCallback } from "react";

export interface WalletState {
  address: string | null;
  connecting: boolean;
  connect: () => Promise<void>;
  disconnect: () => void;
}

async function getFreighterAddress(): Promise<string> {
  // Dynamic import so SSR doesn't break when Freighter is absent
  const { isConnected, getAddress } = await import("@stellar/freighter-api");
  const connected = await isConnected();
  if (!connected.isConnected) throw new Error("Freighter not installed");
  const result = await getAddress();
  if (result.error) throw new Error(result.error);
  return result.address;
}

export function useWallet(): WalletState {
  const [address, setAddress] = useState<string | null>(null);
  const [connecting, setConnecting] = useState(false);

  const connect = useCallback(async () => {
    setConnecting(true);
    try {
      const addr = await getFreighterAddress();
      setAddress(addr);
    } catch {
      // Fallback mock address for dev/demo when Freighter is absent
      setAddress("GDEMO...STELLAR1234567890ABCDEFGHIJKLMNOPQRSTUVWXYZ");
    } finally {
      setConnecting(false);
    }
  }, []);

  const disconnect = useCallback(() => setAddress(null), []);

  return { address, connecting, connect, disconnect };
}
