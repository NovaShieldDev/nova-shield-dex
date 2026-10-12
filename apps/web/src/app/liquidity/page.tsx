"use client";

import { useEffect, useState } from "react";
import { api, type Pool } from "@/lib/api";

type Tab = "add" | "remove" | "position";

export default function LiquidityPage() {
  const [pools, setPools] = useState<Pool[]>([]);
  const [tab, setTab] = useState<Tab>("add");
  const [poolId, setPoolId] = useState("");
  const [provider, setProvider] = useState("");
  const [amountA, setAmountA] = useState("");
  const [amountB, setAmountB] = useState("");
  const [lpToBurn, setLpToBurn] = useState("");
  const [publicKey, setPublicKey] = useState("");
  const [signature, setSignature] = useState("");
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    api.getPools().then(setPools).catch(() => {});
  }, []);

  function reset() { setResult(null); setError(null); }

  async function handleAdd(e: React.FormEvent) {
    e.preventDefault(); reset(); setLoading(true);
    try {
      const res = await api.addLiquidity(poolId, { provider, amount_a: amountA, amount_b: amountB, public_key: publicKey, signature, timestamp_secs: Math.floor(Date.now() / 1000) });
      setResult(`LP token alındı: ${BigInt(res.lp_tokens).toLocaleString()}`);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally { setLoading(false); }
  }

  async function handleRemove(e: React.FormEvent) {
    e.preventDefault(); reset(); setLoading(true);
    try {
      const res = await api.removeLiquidity(poolId, { provider, lp_to_burn: lpToBurn, public_key: publicKey, signature, timestamp_secs: Math.floor(Date.now() / 1000) });
      setResult(`Geri alındı: A=${BigInt(res.amount_a).toLocaleString()}  B=${BigInt(res.amount_b).toLocaleString()} nNOVA`);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally { setLoading(false); }
  }

  async function handlePosition(e: React.FormEvent) {
    e.preventDefault(); reset(); setLoading(true);
    try {
      const pos = await api.getPosition(poolId, provider);
      setResult(
        `LP: ${BigInt(pos.lp_tokens).toLocaleString()} | ` +
        `Dep. A: ${BigInt(pos.deposited_a).toLocaleString()} | ` +
        `Dep. B: ${BigInt(pos.deposited_b).toLocaleString()}`
      );
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally { setLoading(false); }
  }

  const inputStyle: React.CSSProperties = {
    width: "100%", padding: "8px 12px", background: "#0f172a",
    border: "1px solid #1e293b", borderRadius: "6px",
    color: "#e2e8f0", fontSize: "14px", boxSizing: "border-box",
  };
  const labelStyle: React.CSSProperties = {
    display: "block", marginBottom: "4px", fontSize: "13px", color: "#64748b",
  };
  const btnBase: React.CSSProperties = {
    padding: "6px 16px", border: "none", borderRadius: "6px",
    cursor: "pointer", fontSize: "13px",
  };

  const tabs: { key: Tab; label: string }[] = [
    { key: "add", label: "Likidite Ekle" },
    { key: "remove", label: "Likidite Çek" },
    { key: "position", label: "Pozisyon Sorgula" },
  ];

  const onSubmit = tab === "add" ? handleAdd : tab === "remove" ? handleRemove : handlePosition;

  return (
    <div style={{ maxWidth: "480px" }}>
      <h1 style={{ fontSize: "24px", marginBottom: "24px" }}>Likidite</h1>

      <div style={{ display: "flex", gap: "8px", marginBottom: "24px" }}>
        {tabs.map((t) => (
          <button
            key={t.key}
            onClick={() => { setTab(t.key); reset(); }}
            style={{
              ...btnBase,
              background: tab === t.key ? "#4f46e5" : "#1e293b",
              color: tab === t.key ? "#fff" : "#94a3b8",
            }}
          >
            {t.label}
          </button>
        ))}
      </div>

      <form onSubmit={onSubmit} style={{ display: "flex", flexDirection: "column", gap: "16px" }}>
        <div>
          <label style={labelStyle}>Havuz</label>
          <select value={poolId} onChange={(e) => setPoolId(e.target.value)} style={inputStyle} required>
            <option value="">Havuz seçin...</option>
            {pools.map((p) => (
              <option key={p.id} value={p.id}>
                {p.asset_a.slice(0, 8)} / {p.asset_b.slice(0, 8)} — {p.id.slice(0, 12)}...
              </option>
            ))}
          </select>
        </div>

        <div>
          <label style={labelStyle}>Sağlayıcı Adresi (hex)</label>
          <input style={inputStyle} value={provider} onChange={(e) => setProvider(e.target.value)} placeholder="0000...0000" required />
        </div>

        {tab === "add" && (
          <>
            <div>
              <label style={labelStyle}>Miktar A (nNOVA)</label>
              <input style={inputStyle} value={amountA} onChange={(e) => setAmountA(e.target.value)} placeholder="1000000000000000000" required />
            </div>
            <div>
              <label style={labelStyle}>Miktar B (nNOVA)</label>
              <input style={inputStyle} value={amountB} onChange={(e) => setAmountB(e.target.value)} placeholder="1000000000000000000" required />
            </div>
          </>
        )}

        {tab === "remove" && (
          <div>
            <label style={labelStyle}>Yakılacak LP Token</label>
            <input style={inputStyle} value={lpToBurn} onChange={(e) => setLpToBurn(e.target.value)} placeholder="1000000" required />
          </div>
        )}

        {(tab === "add" || tab === "remove") && (
          <>
            <div>
              <label style={labelStyle}>Public Key (hex, 33 byte compressed secp256k1)</label>
              <input style={inputStyle} value={publicKey} onChange={e => setPublicKey(e.target.value)} placeholder="02..." required />
            </div>
            <div>
              <label style={labelStyle}>İmza (hex, 64 byte ECDSA compact)</label>
              <input style={inputStyle} value={signature} onChange={e => setSignature(e.target.value)} placeholder="r||s hex" required />
            </div>
          </>
        )}

        <button
          type="submit"
          disabled={loading}
          style={{
            padding: "10px", background: loading ? "#1e293b" : "#4f46e5",
            color: "#fff", border: "none", borderRadius: "6px",
            cursor: loading ? "not-allowed" : "pointer", fontSize: "15px",
          }}
        >
          {loading ? "İşleniyor..." : tab === "add" ? "Likidite Ekle" : tab === "remove" ? "Likidite Çek" : "Sorgula"}
        </button>
      </form>

      {error && (
        <div style={{ marginTop: "16px", padding: "12px", background: "#1c0000", border: "1px solid #7f1d1d", borderRadius: "6px", color: "#f87171", fontSize: "13px" }}>
          {error}
        </div>
      )}

      {result && (
        <div style={{ marginTop: "16px", padding: "12px", background: "#052e16", border: "1px solid #166534", borderRadius: "6px", color: "#4ade80", fontSize: "13px" }}>
          {result}
        </div>
      )}
    </div>
  );
}
