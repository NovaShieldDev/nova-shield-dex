//! Nova DEX — Kayıt Defteri Anlık Görüntüsü (Snapshot).
//!
//! RocksDB'ye bincode ile yazılır/okunur.
//! `serde` feature aktif olduğunda kullanılabilir.

use serde::{Deserialize, Serialize};

use crate::types::{Amount, Timestamp};

// ─── SNAPSHOT VERSION ────────────────────────────────────────────────────────

pub const SNAPSHOT_VERSION: u32 = 1;

// ─── LOCK DURATION (Snapshot kopyası) ────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum LockDurationSnap {
    OneDay,
    ThreeDays,
    OneWeek,
    OneMonth,
    TwelveMonths,
    Unlimited,
}

impl From<crate::types::LockDuration> for LockDurationSnap {
    fn from(d: crate::types::LockDuration) -> Self {
        match d {
            crate::types::LockDuration::OneDay       => Self::OneDay,
            crate::types::LockDuration::ThreeDays    => Self::ThreeDays,
            crate::types::LockDuration::OneWeek      => Self::OneWeek,
            crate::types::LockDuration::OneMonth     => Self::OneMonth,
            crate::types::LockDuration::TwelveMonths => Self::TwelveMonths,
            crate::types::LockDuration::Unlimited    => Self::Unlimited,
        }
    }
}

impl From<LockDurationSnap> for crate::types::LockDuration {
    fn from(d: LockDurationSnap) -> Self {
        match d {
            LockDurationSnap::OneDay       => Self::OneDay,
            LockDurationSnap::ThreeDays    => Self::ThreeDays,
            LockDurationSnap::OneWeek      => Self::OneWeek,
            LockDurationSnap::OneMonth     => Self::OneMonth,
            LockDurationSnap::TwelveMonths => Self::TwelveMonths,
            LockDurationSnap::Unlimited    => Self::Unlimited,
        }
    }
}

// ─── POOL SNAPSHOT ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct PoolSnap {
    pub id:                   [u8; 32],
    pub asset_a:              [u8; 32],
    pub asset_b:              [u8; 32],
    pub creator:              [u8; 32],
    pub reserve_a:            Amount,
    pub reserve_b:            Amount,
    pub total_lp:             Amount,
    // LockInfo alanları düzleştirildi
    pub lock_creator:         [u8; 32],
    pub lock_duration:        LockDurationSnap,
    pub lock_locked_at:       Timestamp,
    pub lock_unlock_at:       Timestamp,
    // İstatistikler
    pub created_at:           Timestamp,
    pub total_volume_a:       Amount,
    pub total_fees_collected: Amount,
    pub swap_count:           u64,
}

// ─── LP POZİSYON SNAPSHOT ────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct LpPositionSnap {
    pub pool_id:     [u8; 32],
    pub provider:    [u8; 32],
    pub lp_tokens:   Amount,
    pub deposited_a: Amount,
    pub deposited_b: Amount,
    pub created_at:  Timestamp,
}

// ─── REGISTRY SNAPSHOT ───────────────────────────────────────────────────────

/// Tüm PoolRegistry durumunun taşınabilir kopyası.
///
/// RocksDB'de tek bir anahtar altında bincode olarak saklanır.
/// Yeniden başlatmada tam geri yüklemeyi destekler.
#[derive(Debug, Serialize, Deserialize)]
pub struct RegistrySnapshot {
    /// Şema sürümü — ileriki göç (migration) için.
    pub version:          u32,
    pub genesis_time:     Timestamp,
    pub treasury_balance: Amount,
    pub last_payout_at:   Timestamp,
    pub pools:            Vec<PoolSnap>,
    pub lp_positions:     Vec<LpPositionSnap>,
    /// (adres_bytes, günlük_sayı, gün_başlangıcı_saniye)
    pub bot_counters:     Vec<([u8; 32], u64, u64)>,
    /// (creator_bytes, bekleyen_miktar)
    pub pending_payouts:  Vec<([u8; 32], Amount)>,
}
