import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "StellarSwipe",
  description: "Decentralized swipe-to-copy-trade on Stellar",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body style={{ margin: 0, background: "#0d0d1a", color: "#e0e0ff", fontFamily: "sans-serif" }}>
        {children}
      </body>
    </html>
  );
}
