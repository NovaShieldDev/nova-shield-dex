import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Nova DEX",
  description: "Nova Shield Chain — Merkezi olmayan borsa",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="tr">
      <body style={{ margin: 0, fontFamily: "monospace", background: "#0a0a0f", color: "#e2e8f0" }}>
        <header style={{
          padding: "12px 24px",
          borderBottom: "1px solid #1e293b",
          display: "flex",
          alignItems: "center",
          gap: "12px",
        }}>
          <span style={{ fontSize: "20px", fontWeight: "bold", color: "#818cf8" }}>
            ◆ Nova DEX
          </span>
          <nav style={{ marginLeft: "auto", display: "flex", gap: "24px" }}>
            <a href="/" style={{ color: "#94a3b8", textDecoration: "none" }}>Havuzlar</a>
            <a href="/swap" style={{ color: "#94a3b8", textDecoration: "none" }}>Swap</a>
            <a href="/liquidity" style={{ color: "#94a3b8", textDecoration: "none" }}>Likidite</a>
            <a href="/create-pool" style={{ color: "#94a3b8", textDecoration: "none" }}>Havuz Oluştur</a>
            <a href="/manage" style={{ color: "#94a3b8", textDecoration: "none" }}>Yönetim</a>
          </nav>
        </header>
        <main style={{ maxWidth: "960px", margin: "0 auto", padding: "32px 24px" }}>
          {children}
        </main>
      </body>
    </html>
  );
}
