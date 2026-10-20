import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "StellarSwipe",
  description: "Network-aware decentralized swipe-to-copy-trade on Stellar",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en">
      <body>
        {children}
      </body>
    </html>
  );
}
