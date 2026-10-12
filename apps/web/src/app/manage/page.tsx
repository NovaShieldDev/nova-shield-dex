"use client";

import { useEffect, useState } from "react";
import { api, type Pool } from "@/lib/api";

type Tab = "extend-lock" | "payouts";

export default function ManagePage() {
  const [pools, setPools] = useState<Pool[]>([]);
  const [tab, setTab] = useState<Tab>("extend-lock");

  // extend-lock state
  const [lockPoolId, setLockPoolId] = useState("");
  const [caller, setCaller] = useState("");
  const [newDuration, setNewDuration] = useState("one_month");
  const [lockPublicKey, setLockPublicKey] = useState("");
  const [lockSignature, setLockSignature] = useState("");

  // payouts state
  const [payoutResult, setPayoutResult] = useState<{ creator: string; gross_amount: string; net_amount: string; transfer_fee: string; paid_at: number }[]>([]);

  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    api.getPools().then(setPools).catch(() => {});
  }, []);

  function reset() { setResult(null); setError(null); setPayoutResult([]); }

  async function handleExtendLock(e: React.FormEvent) {
    e.preventDefault(); reset(); setLoading(true);
    try {
      await api.extendLock(lockPoolId, { caller, new_duration: newDuration, public_key: lockPublicKey, signature: lockSignature, timestamp_secs: Math.floor(Date.now() / 1000) });
      setResult("Kilit süresi uzatıldı.");
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally { setLoading(false); }
  }

  async function handlePayouts(e: React.FormEvent) {
    e.preventDefault(); reset(); setLoading(true);
    try {
      const res = await api.processPayouts();
      setPayoutResult(res);
      setResult(`${res.length} ödeme işlendi.`);
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

  const tabs: { key: Tab; label: string }[] = [
    { key: "extend-lock", label: "Kilit Uzat" },
    { key: "payouts", label: "Creator Ödemeleri" },
  ];

  return (
    <div style={{ maxWidth: "480px" }}>
      <h1 style={{ fontSize: "24px", marginBottom: "24px" }}>Yönetim</h1>

      <div style={{ display: "flex", gap: "8px", marginBottom: "24px" }}>
        {tabs.map(t => (
          <button
            key={t.key}
            onClick={() => { setTab(t.key); reset(); }}
            style={{
              padding: "6px 16px", border: "none", borderRadius: "6px",
              cursor: "pointer", fontSize: "13px",
              background: tab === t.key ? "#4f46e5" : "#1e293b",
              color: tab === t.key ? "#fff" : "#94a3b8",
            }}
          >
            {t.label}
          </button>
        ))}
      </div>

      {tab === "extend-lock" && (
        <form onSubmit={handleExtendLock} style={{ display: "flex", flexDirection: "column", gap: "16px" }}>
          <div>
            <label style={labelStyle}>Havuz</label>
            <select value={lockPoolId} onChange={e => setLockPoolId(e.target.value)} style={inputStyle} required>
              <option value="">Havuz seçin...</option>
              {pools.map(p => (
                <option key={p.id} value={p.id}>
                  {p.asset_a.slice(0, 8)} / {p.asset_b.slice(0, 8)} — {p.id.slice(0, 12)}...
                </option>
              ))}
            </select>
          </div>
          <div>
            <label style={labelStyle}>Çağıran (Creator) Adresi</label>
            <input style={inputStyle} value={caller} onChange={e => setCaller(e.target.value)} placeholder="0000...0000" required />
          </div>
          <div>
            <label style={labelStyle}>Yeni Kilit Süresi</label>
            <select value={newDuration} onChange={e => setNewDuration(e.target.value)} style={inputStyle}>
              <option value="one_day">1 Gün</option>
              <option value="three_days">3 Gün</option>
              <option value="one_week">1 Hafta</option>
              <option value="one_month">1 Ay</option>
              <option value="twelve_months">12 Ay</option>
              <option value="unlimited">Sınırsız (Kalıcı)</option>
            </select>
          </div>
          <div>
            <label style={labelStyle}>Public Key (hex, 33 byte compressed secp256k1)</label>
            <input style={inputStyle} value={lockPublicKey} onChange={e => setLockPublicKey(e.target.value)} placeholder="02..." required />
          </div>
          <div>
            <label style={labelStyle}>İmza (hex, 64 byte ECDSA compact)</label>
            <input style={inputStyle} value={lockSignature} onChange={e => setLockSignature(e.target.value)} placeholder="r||s hex" required />
          </div>
          <button
            type="submit" disabled={loading}
            style={{ padding: "10px", background: loading ? "#1e293b" : "#4f46e5", color: "#fff", border: "none", borderRadius: "6px", cursor: loading ? "not-allowed" : "pointer", fontSize: "15px" }}
          >
            {loading ? "İşleniyor..." : "Kilidi Uzat"}
          </button>
        </form>
      )}

      {tab === "payouts" && (
        <form onSubmit={handlePayouts} style={{ display: "flex", flexDirection: "column", gap: "16px" }}>
          <p style={{ margin: 0, fontSize: "13px", color: "#64748b" }}>
            24 saatlik döngü içinde birikmiş creator ödemelerini tetikler.
            Tüm havuzlar taranır, hak kazanan creator'lara ödeme yapılır.
          </p>
          <button
            type="submit" disabled={loading}
            style={{ padding: "10px", background: loading ? "#1e293b" : "#4f46e5", color: "#fff", border: "none", borderRadius: "6px", cursor: loading ? "not-allowed" : "pointer", fontSize: "15px" }}
          >
            {loading ? "İşleniyor..." : "Ödemeleri Tetikle"}
          </button>
        </form>
      )}

      {error && (
        <div style={{ marginTop: "16px", padding: "12px", background: "#1c0000", border: "1px solid #7f1d1d", borderRadius: "6px", color: "#f87171", fontSize: "13px" }}>
          {error}
        </div>
      )}

      {result && (
        <div style={{ marginTop: "16px", padding: "12px", background: "#052e16", border: "1px solid #166534", borderRadius: "6px", color: "#4ade80", fontSize: "13px" }}>
          {result}
          {payoutResult.length > 0 && (
            <table style={{ marginTop: "12px", width: "100%", borderCollapse: "collapse", fontSize: "12px" }}>
              <thead>
                <tr>
                  <th style={{ textAlign: "left", color: "#64748b", paddingBottom: "4px" }}>Alıcı</th>
                  <th style={{ textAlign: "right", color: "#64748b", paddingBottom: "4px" }}>Miktar (nNOVA)</th>
                </tr>
              </thead>
              <tbody>
                {payoutResult.map((p, i) => (
                  <tr key={i}>
                    <td style={{ color: "#94a3b8", paddingRight: "8px" }}>{p.creator.slice(0, 12)}...</td>
                    <td style={{ textAlign: "right", color: "#e2e8f0" }}>{BigInt(p.net_amount).toLocaleString()}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      )}
    </div>
  );
}
