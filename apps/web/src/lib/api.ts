// Nova DEX API istemcisi

const BASE = process.env.NEXT_PUBLIC_DEX_API ?? "http://localhost:3000";

export interface Pool {
  id: string;
  asset_a: string;
  asset_b: string;
  creator: string;
  reserve_a: string;
  reserve_b: string;
  total_lp: string;
  is_permanent: boolean;
  lock_duration: string;
  lock_status: string;
  unlock_at: number;
  swap_count: number;
  total_volume_a: string;
  total_fees_collected: string;
  created_at: number;
}

export interface SwapResult {
  amount_out: string;
  amount_in_net: string;
  fee_amount: string;
  creator_fee: string;
  treasury_fee: string;
  fee_mode: string;
  price_impact_bps: number;
}

export interface LpPosition {
  provider: string;
  lp_tokens: string;
  deposited_a: string;
  deposited_b: string;
  created_at: number;
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    headers: { "Content-Type": "application/json" },
    ...init,
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`${res.status}: ${text}`);
  }
  return res.json() as Promise<T>;
}

export const api = {
  health: () => request<{ status: string; version: string }>("/health"),

  getPools: () => request<Pool[]>("/v1/pools"),

  getPool: (id: string) => request<Pool>(`/v1/pools/${id}`),

  createPool: (body: {
    creator: string;
    asset_a: string;
    asset_b: string;
    initial_a: string;
    initial_b: string;
    lock_duration: string;
    creation_fee_paid: string;
    public_key: string;
    signature: string;
    timestamp_secs: number;
  }) =>
    request<{ pool_id: string }>("/v1/pools", {
      method: "POST",
      body: JSON.stringify(body),
    }),

  mevCommit: (body: { commitment_hash: string; sender: string }) =>
    request<{ slot_id: number }>("/v1/mev/commit", {
      method: "POST",
      body: JSON.stringify(body),
    }),

  mevReveal: (body: {
    sender: string;
    pool_id: string;
    asset_in: string;
    asset_out: string;
    amount_in: string;
    min_amount_out: string;
    nonce: string;
  }) =>
    request<SwapResult>("/v1/mev/reveal", {
      method: "POST",
      body: JSON.stringify(body),
    }),

  addLiquidity: (
    poolId: string,
    body: {
      provider: string;
      amount_a: string;
      amount_b: string;
      public_key: string;
      signature: string;
      timestamp_secs: number;
    }
  ) =>
    request<{ lp_tokens: string; actual_a: string; actual_b: string }>(`/v1/pools/${poolId}/liquidity`, {
      method: "POST",
      body: JSON.stringify(body),
    }),

  removeLiquidity: (
    poolId: string,
    body: {
      provider: string;
      lp_to_burn: string;
      public_key: string;
      signature: string;
      timestamp_secs: number;
    }
  ) =>
    request<{ amount_a: string; amount_b: string }>(
      `/v1/pools/${poolId}/liquidity`,
      { method: "DELETE", body: JSON.stringify(body) }
    ),

  getPosition: (poolId: string, address: string) =>
    request<LpPosition>(`/v1/pools/${poolId}/lp/${address}`),

  extendLock: (poolId: string, body: { caller: string; new_duration: string; public_key: string; signature: string; timestamp_secs: number }) =>
    request<{ ok: boolean }>(`/v1/pools/${poolId}/lock/extend`, {
      method: "POST",
      body: JSON.stringify(body),
    }),

  processPayouts: () =>
    request<{ creator: string; gross_amount: string; net_amount: string; transfer_fee: string; paid_at: number }[]>(
      "/v1/payouts/process",
      { method: "POST" }
    ),
};
