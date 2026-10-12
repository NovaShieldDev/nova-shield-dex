use nova_dex::types::{FeeMode, LockDuration};
use serde::{Deserialize, Serialize};

// ─── LockDuration ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum LockDurationDto {
    OneDay,
    ThreeDays,
    OneWeek,
    OneMonth,
    TwelveMonths,
    Unlimited,
}

impl From<LockDurationDto> for LockDuration {
    fn from(d: LockDurationDto) -> Self {
        match d {
            LockDurationDto::OneDay       => LockDuration::OneDay,
            LockDurationDto::ThreeDays    => LockDuration::ThreeDays,
            LockDurationDto::OneWeek      => LockDuration::OneWeek,
            LockDurationDto::OneMonth     => LockDuration::OneMonth,
            LockDurationDto::TwelveMonths => LockDuration::TwelveMonths,
            LockDurationDto::Unlimited    => LockDuration::Unlimited,
        }
    }
}

impl From<LockDuration> for LockDurationDto {
    fn from(d: LockDuration) -> Self {
        match d {
            LockDuration::OneDay       => LockDurationDto::OneDay,
            LockDuration::ThreeDays    => LockDurationDto::ThreeDays,
            LockDuration::OneWeek      => LockDurationDto::OneWeek,
            LockDuration::OneMonth     => LockDurationDto::OneMonth,
            LockDuration::TwelveMonths => LockDurationDto::TwelveMonths,
            LockDuration::Unlimited    => LockDurationDto::Unlimited,
        }
    }
}

// ─── FeeMode ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeeModeDto {
    Normal,
    Bot,
}

impl From<FeeMode> for FeeModeDto {
    fn from(f: FeeMode) -> Self {
        match f {
            FeeMode::Normal => FeeModeDto::Normal,
            FeeMode::Bot    => FeeModeDto::Bot,
        }
    }
}

// ─── Pool ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreatePoolRequest {
    pub creator:           String,
    pub asset_a:           String,
    pub asset_b:           String,
    pub initial_a:         String,
    pub initial_b:         String,
    pub lock_duration:     LockDurationDto,
    pub creation_fee_paid: String,
    /// secp256k1 compressed public key (hex, 33 byte = 66 karakter)
    pub public_key:        String,
    /// ECDSA compact signature (hex, 64 byte = 128 karakter)
    pub signature:         String,
    /// İstemci tarafı Unix timestamp (saniye) — replay koruması için
    pub timestamp_secs:    u64,
}

#[derive(Debug, Serialize)]
pub struct CreatePoolResponse {
    pub pool_id: String,
}

#[derive(Debug, Serialize)]
pub struct PoolResponse {
    pub id:                   String,
    pub asset_a:              String,
    pub asset_b:              String,
    pub creator:              String,
    pub reserve_a:            String,
    pub reserve_b:            String,
    pub total_lp:             String,
    pub is_permanent:         bool,
    pub lock_duration:        LockDurationDto,
    pub lock_status:          String,
    pub unlock_at:            u64,
    pub swap_count:           u64,
    pub total_volume_a:       String,
    pub total_fees_collected: String,
    pub created_at:           u64,
}

// ─── Swap ────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SwapResponse {
    pub amount_out:       String,
    pub amount_in_net:    String,
    pub fee_amount:       String,
    pub creator_fee:      String,
    pub treasury_fee:     String,
    pub fee_mode:         FeeModeDto,
    pub price_impact_bps: u128,
}

// ─── Likidite ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AddLiquidityRequest {
    pub provider:       String,
    pub amount_a:       String,
    pub amount_b:       String,
    pub public_key:     String,
    pub signature:      String,
    pub timestamp_secs: u64,
}

#[derive(Debug, Serialize)]
pub struct AddLiquidityResponse {
    pub lp_tokens: String,
    pub actual_a:  String,
    pub actual_b:  String,
}

#[derive(Debug, Deserialize)]
pub struct RemoveLiquidityRequest {
    pub provider:       String,
    pub lp_to_burn:     String,
    pub public_key:     String,
    pub signature:      String,
    pub timestamp_secs: u64,
}

#[derive(Debug, Serialize)]
pub struct RemoveLiquidityResponse {
    pub amount_a: String,
    pub amount_b: String,
}

#[derive(Debug, Deserialize)]
pub struct ExtendLockRequest {
    pub caller:         String,
    pub new_duration:   LockDurationDto,
    pub public_key:     String,
    pub signature:      String,
    pub timestamp_secs: u64,
}

#[derive(Debug, Serialize)]
pub struct LpPositionResponse {
    pub provider:    String,
    pub lp_tokens:   String,
    pub deposited_a: String,
    pub deposited_b: String,
    pub created_at:  u64,
}

// ─── Payout ──────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PayoutRecordResponse {
    pub creator:      String,
    pub gross_amount: String,
    pub net_amount:   String,
    pub transfer_fee: String,
    pub paid_at:      u64,
}

// ─── MEV ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CommitRequest {
    pub commitment_hash: String,
    pub sender:          String,
}

#[derive(Debug, Serialize)]
pub struct CommitResponse {
    pub slot_id: u64,
}

#[derive(Debug, Deserialize)]
pub struct RevealRequest {
    pub sender:         String,
    pub pool_id:        String,
    pub asset_in:       String,
    pub asset_out:      String,
    pub amount_in:      String,
    pub min_amount_out: String,
    pub nonce:          String,
    /// AUTH-001: secp256k1 compressed public key (hex, 33 byte)
    pub public_key:     String,
    /// AUTH-001: ECDSA compact signature (hex, 64 byte)
    pub signature:      String,
    /// Replay koruması için istemci zaman damgası (Unix saniye)
    pub timestamp_secs: u64,
}

#[derive(Debug, Serialize)]
pub struct SwapIntentResponse {
    pub sender:         String,
    pub pool_id:        String,
    pub asset_in:       String,
    pub asset_out:      String,
    pub amount_in:      String,
    pub min_amount_out: String,
    pub nonce:          String,
}

// ─── Health ──────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status:     &'static str,
    pub pool_count: usize,
    pub version:    &'static str,
}
