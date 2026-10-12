"use client";

import { useState } from "react";
import { api } from "@/lib/api";

export default function CreatePoolPage() {
  const [creator, setCreator] = useState("");
  const [assetA, setAssetA] = useState("");
  const [assetB, setAssetB] = useState("");
  const [initialA, setInitialA] = useState("");
  const [initialB, setInitialB] = useState("");
  const [lockDuration, setLockDuration] = useState("one_day");
  const [creationFee, setCreationFee] = useState("0");
  const [publicKey, setPublicKey] = useState("");
  const [signature, setSignature] = useState("");
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setResult(null);
    setLoading(true);
    try {
      const res = await api.createPool({
        creator,
        asset_a: assetA,
        asset_b: assetB,
        initial_a: initialA,
        initial_b: initialB,
        lock_duration: lockDuration,
        creation_fee_paid: creationFee,
        public_key: publicKey,
        signature,
        timestamp_secs: Math.floor(Date.now() / 1000),
      });
      setResult(`Havuz oluşturuldu! ID: ${res.pool_id}`);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }

  const inputStyle: React.CSSProperties = {
    width: "100%",
    padding: "8px 12px",
    background: "#0f172a",
    border: "1px solid #1e293b",
    borderRadius: "6px",
    color: "#e2e8f0",
    fontSize: "14px",
    boxSizing: "border-box",
  };
  const labelStyle: React.CSSProperties = {
    display: "block",
    marginBottom: "4px",
    fontSize: "13px",
    color: "#64748b",
  };

  return (
    <div style={{ maxWidth: "480px" }}>
      <h1 style={{ fontSize: "24px", marginBottom: "24px" }}>Havuz Oluştur</h1>

      <form onSubmit={handleSubmit} style={{ display: "flex", flexDirection: "column", gap: "16px" }}>
        <div>
          <label style={labelStyle}>Creator Adresi (hex)</label>
          <input style={inputStyle} value={creator} onChange={e => setCreator(e.target.value)} placeholder="0000...0000" required />
        </div>

        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "12px" }}>
          <div>
            <label style={labelStyle}>Asset A (hex)</label>
            <input style={inputStyle} value={assetA} onChange={e => setAssetA(e.target.value)} placeholder="token A id" required />
          </div>
          <div>
            <label style={labelStyle}>Asset B (hex)</label>
            <input style={inputStyle} value={assetB} onChange={e => setAssetB(e.target.value)} placeholder="token B id" required />
          </div>
        </div>

        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "12px" }}>
          <div>
            <label style={labelStyle}>Başlangıç A (nNOVA)</label>
            <input style={inputStyle} value={initialA} onChange={e => setInitialA(e.target.value)} placeholder="1000000000" required />
          </div>
          <div>
            <label style={labelStyle}>Başlangıç B (nNOVA)</label>
            <input style={inputStyle} value={initialB} onChange={e => setInitialB(e.target.value)} placeholder="1000000000" required />
          </div>
        </div>

        <div>
          <label style={labelStyle}>Kilit Süresi</label>
          <select value={lockDuration} onChange={e => setLockDuration(e.target.value)} style={inputStyle}>
            <option value="one_day">1 Gün</option>
            <option value="three_days">3 Gün</option>
            <option value="one_week">1 Hafta</option>
            <option value="one_month">1 Ay</option>
            <option value="twelve_months">12 Ay</option>
            <option value="unlimited">Sınırsız (Kalıcı)</option>
          </select>
        </div>

        <div>
          <label style={labelStyle}>Oluşturma Ücreti (nNOVA)</label>
          <input style={inputStyle} value={creationFee} onChange={e => setCreationFee(e.target.value)} placeholder="0" required />
        </div>

        <div>
          <label style={labelStyle}>Public Key (hex, 33 byte compressed secp256k1)</label>
          <input style={inputStyle} value={publicKey} onChange={e => setPublicKey(e.target.value)} placeholder="02..." required />
        </div>

        <div>
          <label style={labelStyle}>İmza (hex, 64 byte ECDSA compact)</label>
          <input style={inputStyle} value={signature} onChange={e => setSignature(e.target.value)} placeholder="r||s hex" required />
        </div>

        <button
          type="submit"
          disabled={loading}
          style={{
            padding: "10px",
            background: loading ? "#1e293b" : "#4f46e5",
            color: "#fff",
            border: "none",
            borderRadius: "6px",
            cursor: loading ? "not-allowed" : "pointer",
            fontSize: "15px",
          }}
        >
          {loading ? "Oluşturuluyor..." : "Havuz Oluştur"}
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
