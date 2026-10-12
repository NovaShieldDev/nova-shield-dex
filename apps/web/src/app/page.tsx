"use client";

import { useEffect, useState } from "react";
import { api, type Pool } from "@/lib/api";

function PoolCard({ pool }: { pool: Pool }) {
  const shortId = pool.id.slice(0, 8) + "...";
  const assetA = pool.asset_a.slice(0, 8);
  const assetB = pool.asset_b.slice(0, 8);
  const isLocked = pool.lock_status !== "Unlocked";

  return (
    <div style={{
      border: "1px solid #1e293b",
      borderRadius: "8px",
      padding: "16px",
      background: "#0f172a",
    }}>
      <div style={{ display: "flex", justifyContent: "space-between", marginBottom: "8px" }}>
        <span style={{ fontWeight: "bold", color: "#818cf8" }}>
          {assetA} / {assetB}
        </span>
        <span style={{
          fontSize: "12px",
          padding: "2px 8px",
          borderRadius: "4px",
          background: isLocked ? "#1e293b" : "#052e16",
          color: isLocked ? "#94a3b8" : "#4ade80",
        }}>
          {pool.lock_status}
        </span>
      </div>
      <div style={{ fontSize: "13px", color: "#64748b", lineHeight: "1.6" }}>
        <div>Rezerv A: <span style={{ color: "#e2e8f0" }}>{BigInt(pool.reserve_a).toLocaleString()} nNOVA</span></div>
        <div>Rezerv B: <span style={{ color: "#e2e8f0" }}>{BigInt(pool.reserve_b).toLocaleString()} nNOVA</span></div>
        <div>LP Arzı: <span style={{ color: "#e2e8f0" }}>{BigInt(pool.total_lp).toLocaleString()}</span></div>
        <div>İşlem: <span style={{ color: "#e2e8f0" }}>{pool.swap_count}</span></div>
        <div>Kilit: <span style={{ color: "#e2e8f0" }}>{pool.lock_duration}</span></div>
        <div style={{ marginTop: "8px", fontSize: "11px", color: "#475569" }}>ID: {shortId}</div>
      </div>
    </div>
  );
}

export default function HomePage() {
  const [pools, setPools] = useState<Pool[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [health, setHealth] = useState<string>("kontrol ediliyor...");

  useEffect(() => {
    api.health()
      .then((h) => setHealth(`${h.status} (v${h.version})`))
      .catch(() => setHealth("bağlanamadı"));

    api.getPools()
      .then((p) => { setPools(p); setLoading(false); })
      .catch((e: Error) => { setError(e.message); setLoading(false); });
  }, []);

  return (
    <div>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: "24px" }}>
        <h1 style={{ margin: 0, fontSize: "24px" }}>Likidite Havuzları</h1>
        <span style={{
          fontSize: "12px",
          padding: "4px 10px",
          borderRadius: "4px",
          background: "#1e293b",
          color: health.startsWith("ok") ? "#4ade80" : "#f87171",
        }}>
          API: {health}
        </span>
      </div>

      {loading && (
        <p style={{ color: "#64748b" }}>Yükleniyor...</p>
      )}

      {error && (
        <div style={{
          padding: "12px 16px",
          borderRadius: "8px",
          background: "#1c0000",
          border: "1px solid #7f1d1d",
          color: "#f87171",
          marginBottom: "16px",
        }}>
          Havuzlar yüklenemedi: {error}
        </div>
      )}

      {!loading && !error && pools.length === 0 && (
        <p style={{ color: "#64748b" }}>Henüz havuz yok.</p>
      )}

      <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(280px, 1fr))", gap: "16px" }}>
        {pools.map((pool) => (
          <PoolCard key={pool.id} pool={pool} />
        ))}
      </div>
    </div>
  );
}
