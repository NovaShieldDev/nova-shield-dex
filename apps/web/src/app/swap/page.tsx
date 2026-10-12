"use client";

import { useEffect, useState } from "react";
import { api, type Pool, type SwapResult } from "@/lib/api";

// ─── Yardımcı fonksiyonlar ────────────────────────────────────────────────────

function hexToBytes(hex: string): Uint8Array {
  const h = hex.replace(/^0x/, "").padStart(64, "0");
  const bytes = new Uint8Array(32);
  for (let i = 0; i < 32; i++) {
    bytes[i] = parseInt(h.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

function u128LeBytes(value: bigint): Uint8Array {
  const bytes = new Uint8Array(16);
  let v = value;
  const mask = BigInt(0xff);
  for (let i = 0; i < 16; i++) {
    bytes[i] = Number(v & mask);
    v >>= BigInt(8);
  }
  return bytes;
}

async function computeCommitmentHash(
  sender: string,
  poolId: string,
  assetIn: string,
  assetOut: string,
  amountIn: bigint,
  minOut: bigint,
  nonce: Uint8Array
): Promise<Uint8Array> {
  const prefix = new TextEncoder().encode("nova_dex_swap_intent_v1:");
  const parts = [
    prefix,
    hexToBytes(sender),
    hexToBytes(poolId),
    hexToBytes(assetIn),
    hexToBytes(assetOut),
    u128LeBytes(amountIn),
    u128LeBytes(minOut),
    nonce,
  ];
  const total = parts.reduce((s, p) => s + p.length, 0);
  const buf = new Uint8Array(total);
  let offset = 0;
  for (const p of parts) { buf.set(p, offset); offset += p.length; }
  const digest = await crypto.subtle.digest("SHA-256", buf);
  return new Uint8Array(digest);
}

function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes).map(b => b.toString(16).padStart(2, "0")).join("");
}

// ─── Bileşen ──────────────────────────────────────────────────────────────────

export default function SwapPage() {
  const [pools, setPools] = useState<Pool[]>([]);
  const [selectedPool, setSelectedPool] = useState("");
  const [swapper, setSwapper] = useState("");
  const [zeroForOne, setZeroForOne] = useState(true);
  const [amountIn, setAmountIn] = useState("");
  const [minOut, setMinOut] = useState("0");
  const [result, setResult] = useState<SwapResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [step, setStep] = useState<string | null>(null);

  useEffect(() => {
    api.getPools().then(setPools).catch(() => {});
  }, []);

  async function handleSwap(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setResult(null);
    setStep(null);
    setLoading(true);

    try {
      const pool = pools.find(p => p.id === selectedPool);
      if (!pool) throw new Error("Havuz bulunamadı");

      const assetIn  = zeroForOne ? pool.asset_a : pool.asset_b;
      const assetOut = zeroForOne ? pool.asset_b : pool.asset_a;

      const amountInBig = BigInt(amountIn);
      const minOutBig   = BigInt(minOut);

      // 1. Rastgele nonce üret
      const nonce = new Uint8Array(32);
      crypto.getRandomValues(nonce);

      // 2. Commitment hash hesapla
      setStep("Commitment hash hesaplanıyor...");
      const hashBytes = await computeCommitmentHash(
        swapper, selectedPool, assetIn, assetOut,
        amountInBig, minOutBig, nonce
      );
      const commitmentHash = bytesToHex(hashBytes);
      const nonceHex = bytesToHex(nonce);

      // 3. Commit gönder
      setStep("Commit gönderiliyor...");
      await api.mevCommit({ commitment_hash: commitmentHash, sender: swapper });

      // 4. Slot kapanmasını bekle (>400ms)
      setStep("Slot kapanması bekleniyor...");
      await new Promise(r => setTimeout(r, 500));

      // 5. Reveal gönder
      setStep("Reveal gönderiliyor...");
      const res = await api.mevReveal({
        sender:         swapper,
        pool_id:        selectedPool,
        asset_in:       assetIn,
        asset_out:      assetOut,
        amount_in:      amountIn,
        min_amount_out: minOut,
        nonce:          nonceHex,
      });

      setResult(res);
      setStep(null);
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
      <h1 style={{ fontSize: "24px", marginBottom: "8px" }}>Swap</h1>
      <p style={{ fontSize: "12px", color: "#475569", marginBottom: "24px" }}>
        MEV korumalı commit-reveal protokolü kullanılır.
      </p>

      <form onSubmit={handleSwap} style={{ display: "flex", flexDirection: "column", gap: "16px" }}>
        <div>
          <label style={labelStyle}>Havuz</label>
          <select
            value={selectedPool}
            onChange={(e) => setSelectedPool(e.target.value)}
            style={inputStyle}
            required
          >
            <option value="">Havuz seçin...</option>
            {pools.map((p) => (
              <option key={p.id} value={p.id}>
                {p.asset_a.slice(0, 8)} / {p.asset_b.slice(0, 8)} — {p.id.slice(0, 12)}...
              </option>
            ))}
          </select>
        </div>

        <div>
          <label style={labelStyle}>Gönderen Adresi (hex)</label>
          <input
            style={inputStyle}
            value={swapper}
            onChange={(e) => setSwapper(e.target.value)}
            placeholder="0000...0000"
            required
          />
        </div>

        <div>
          <label style={labelStyle}>Yön</label>
          <select
            value={zeroForOne ? "true" : "false"}
            onChange={(e) => setZeroForOne(e.target.value === "true")}
            style={inputStyle}
          >
            <option value="true">Asset A → Asset B</option>
            <option value="false">Asset B → Asset A</option>
          </select>
        </div>

        <div>
          <label style={labelStyle}>Miktar (nNOVA)</label>
          <input
            style={inputStyle}
            value={amountIn}
            onChange={(e) => setAmountIn(e.target.value)}
            placeholder="1000000000000000000"
            required
          />
        </div>

        <div>
          <label style={labelStyle}>Min. Çıkış (nNOVA)</label>
          <input
            style={inputStyle}
            value={minOut}
            onChange={(e) => setMinOut(e.target.value)}
            placeholder="0"
          />
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
          {loading ? "İşleniyor..." : "Swap Yap"}
        </button>
      </form>

      {step && (
        <div style={{ marginTop: "16px", padding: "12px", background: "#0f172a", border: "1px solid #1e293b", borderRadius: "6px", color: "#94a3b8", fontSize: "13px" }}>
          {step}
        </div>
      )}

      {error && (
        <div style={{ marginTop: "16px", padding: "12px", background: "#1c0000", border: "1px solid #7f1d1d", borderRadius: "6px", color: "#f87171", fontSize: "13px" }}>
          {error}
        </div>
      )}

      {result && (
        <div style={{ marginTop: "16px", padding: "16px", background: "#052e16", border: "1px solid #166534", borderRadius: "6px", fontSize: "13px", lineHeight: "1.8" }}>
          <div style={{ color: "#4ade80", fontWeight: "bold", marginBottom: "8px" }}>Swap Başarılı</div>
          <div>Çıkış: <strong>{BigInt(result.amount_out).toLocaleString()}</strong> nNOVA</div>
          <div>Net Giriş: {BigInt(result.amount_in_net).toLocaleString()} nNOVA</div>
          <div>Toplam Ücret: {BigInt(result.fee_amount).toLocaleString()} nNOVA</div>
          <div>Creator Payı: {BigInt(result.creator_fee).toLocaleString()} nNOVA</div>
          <div>Hazine Payı: {BigInt(result.treasury_fee).toLocaleString()} nNOVA</div>
          <div>Fiyat Etkisi: {result.price_impact_bps} bps</div>
          <div>Ücret Modu: {result.fee_mode}</div>
        </div>
      )}
    </div>
  );
}
