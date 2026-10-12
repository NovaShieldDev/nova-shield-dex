//! Nova DEX — Temel veri tipleri ve protokol sabitleri.
//!
//! Tüm tipler Nova Shield Chain'e özgüdür; dış zincirlerle bağlantı yoktur.
//!
//! - `Address`  : 32 byte (Nova native adres)
//! - `AssetId`  : 32 byte (token kimliği)
//! - `PoolId`   : 32 byte (SHA-256 türetilmiş)
//! - `Amount`   : u128 (nNOVA — 10^-18 NOVA)
//! - `Timestamp`: u64 (Unix saniye)

use std::fmt;

use sha2::{Digest, Sha256};

// ─── PROTOCOL SABİTLERİ ───────────────────────────────────────────────────────

/// 1 NOVA = 10^18 nNOVA — Nova Shield Chain native token birimi.
pub const NOVA_DECIMALS_FACTOR: u128 = 1_000_000_000_000_000_000u128;

/// Pool oluşturma hedef ücreti: $17.50 = 1750 cent.
pub const POOL_CREATION_FEE_USD_CENTS: u64 = 1_750;

/// Günlük 100+ işlemde bot fee modu tetiklenir.
pub const BOT_TX_THRESHOLD: u64 = 100;

/// Creator swap fee payı: %20.
pub const CREATOR_FEE_SHARE_NUMERATOR: u128 = 20;
pub const CREATOR_FEE_SHARE_DENOMINATOR: u128 = 100;

/// Kasa (treasury) swap fee payı: %80.
pub const TREASURY_FEE_SHARE_NUMERATOR: u128 = 80;
pub const TREASURY_FEE_SHARE_DENOMINATOR: u128 = 100;

/// Varsayılan slippage toleransı: %5 = 500 bps.
pub const DEFAULT_SLIPPAGE_BPS: u128 = 500;

/// Creator ödeme döngüsü: 24 saat.
pub const PAYOUT_CYCLE_SECS: u64 = 86_400;

/// UTC gün uzunluğu: 24 saat.
pub const UTC_DAY_SECS: u64 = 86_400;

/// Payout işlem transfer ücreti: 0.001 NOVA = 10^15 nNOVA.
pub const PAYOUT_TRANSFER_FEE_NNOVA: u128 = 1_000_000_000_000_000u128;

// ─── AMOUNT & TIMESTAMP ──────────────────────────────────────────────────────

/// Nano-NOVA (nNOVA) — NOVA token'ının atomik birimi (10^-18 NOVA).
pub type Amount = u128;

/// Unix zaman damgası (saniye).
pub type Timestamp = u64;

// ─── ADDRESS ─────────────────────────────────────────────────────────────────

/// Nova Shield hesap adresi — 32 byte, Nova'ya özgü format.
///
/// MPC public key'den türetilir: SHA256("nova_addr:" || pubkey_bytes)
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Address(pub [u8; 32]);

impl Address {
    pub const ZERO: Self = Self([0u8; 32]);

    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Public key'den Nova adres türet.
    pub fn from_pubkey(pubkey_bytes: &[u8]) -> Self {
        let mut h = Sha256::new();
        h.update(b"nova_addr:");
        h.update(pubkey_bytes);
        let r = h.finalize();
        let mut b = [0u8; 32];
        b.copy_from_slice(&r);
        Self(b)
    }

    /// Test yardımcısı: u64 tohumundan adres üret.
    pub fn from_seed(n: u64) -> Self {
        let mut b = [0u8; 32];
        b[..8].copy_from_slice(&n.to_le_bytes());
        Self(b)
    }

    /// Bech32 benzeri Nova prefix ile görüntüle.
    pub fn to_nova_addr(&self) -> String {
        format!("nova1{}", hex::encode(&self.0[..20]))
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "nova1{}", hex::encode(&self.0[..20]))
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Address(nova1{}…)", hex::encode(&self.0[..4]))
    }
}

// ─── ASSET ID ────────────────────────────────────────────────────────────────

/// Token/varlık kimliği — 32 byte, Nova'ya özgü.
/// NOVA native token için `AssetId::NOVA` kullanılır.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetId(pub [u8; 32]);

impl AssetId {
    /// NOVA native token sabit kimliği (sıfır hash).
    pub const NOVA: Self = Self([0u8; 32]);

    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Token adından deterministik AssetId türet: SHA256("nova_asset:" + name).
    pub fn from_name(name: &str) -> Self {
        let input = format!("nova_asset:{name}");
        let h = Sha256::digest(input.as_bytes());
        let mut b = [0u8; 32];
        b.copy_from_slice(&h);
        Self(b)
    }

    /// Test yardımcısı.
    pub fn from_seed(n: u64) -> Self {
        let mut b = [0u8; 32];
        b[..8].copy_from_slice(&n.to_le_bytes());
        Self(b)
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "asset:{}", hex::encode(&self.0[..8]))
    }
}

impl fmt::Debug for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssetId({}…)", hex::encode(&self.0[..4]))
    }
}

// ─── POOL ID ─────────────────────────────────────────────────────────────────

/// Pool kimliği — 32 byte SHA-256 hash, token çiftinden deterministik türetilir.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PoolId(pub [u8; 32]);

impl PoolId {
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Token çiftinden order-independent deterministik pool ID türet.
    pub fn from_assets(a: &AssetId, b: &AssetId) -> Self {
        let (first, second) = if a.0 <= b.0 { (&a.0, &b.0) } else { (&b.0, &a.0) };
        let mut h = Sha256::new();
        h.update(b"nova_pool:");
        h.update(first);
        h.update(second);
        let result = h.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        Self(bytes)
    }
}

impl fmt::Display for PoolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "pool:{}", hex::encode(&self.0[..8]))
    }
}

impl fmt::Debug for PoolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PoolId({}…)", hex::encode(&self.0[..4]))
    }
}

// ─── LOCK DURATION ───────────────────────────────────────────────────────────

/// Likidite kilitleme süresi seçenekleri.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LockDuration {
    OneDay,
    ThreeDays,
    OneWeek,
    OneMonth,
    TwelveMonths,
    Unlimited,
}

impl LockDuration {
    /// Kilitleme süresini saniye cinsinden döner. `Unlimited` → `u64::MAX`.
    pub fn to_seconds(self) -> u64 {
        match self {
            Self::OneDay       => 86_400,
            Self::ThreeDays    => 259_200,
            Self::OneWeek      => 604_800,
            Self::OneMonth     => 2_592_000,
            Self::TwelveMonths => 31_536_000,
            Self::Unlimited    => u64::MAX,
        }
    }

    pub fn is_unlimited(self) -> bool {
        matches!(self, Self::Unlimited)
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::OneDay       => "1 Gün",
            Self::ThreeDays    => "3 Gün",
            Self::OneWeek      => "1 Hafta",
            Self::OneMonth     => "1 Ay",
            Self::TwelveMonths => "12 Ay",
            Self::Unlimited    => "Sınırsız (Permanent Liquidity)",
        }
    }
}

// ─── FEE MODE ────────────────────────────────────────────────────────────────

/// Swap fee modu — günlük işlem sayısına göre belirlenir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeeMode {
    /// 1–99 günlük işlem: %0.30 fee (30 bps).
    Normal,
    /// 100+ günlük işlem: %0.15 fee (15 bps) — bot indirimi.
    Bot,
}

impl FeeMode {
    pub fn fee_bps(self) -> u128 {
        match self {
            Self::Normal => 30,
            Self::Bot    => 15,
        }
    }

    pub fn from_daily_count(count: u64) -> Self {
        if count >= BOT_TX_THRESHOLD { Self::Bot } else { Self::Normal }
    }
}

// ─── TESTS ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_id_order_independent() {
        let a = AssetId::from_name("NOVA");
        let b = AssetId::from_name("USDC");
        assert_eq!(PoolId::from_assets(&a, &b), PoolId::from_assets(&b, &a));
    }

    #[test]
    fn pool_id_different_for_different_pairs() {
        let nova = AssetId::from_name("NOVA");
        let usdc = AssetId::from_name("USDC");
        let btc  = AssetId::from_name("BTC");
        assert_ne!(PoolId::from_assets(&nova, &usdc), PoolId::from_assets(&nova, &btc));
    }

    #[test]
    fn lock_duration_seconds() {
        assert_eq!(LockDuration::OneDay.to_seconds(), 86_400);
        assert_eq!(LockDuration::ThreeDays.to_seconds(), 259_200);
        assert_eq!(LockDuration::OneWeek.to_seconds(), 604_800);
        assert_eq!(LockDuration::OneMonth.to_seconds(), 2_592_000);
        assert_eq!(LockDuration::TwelveMonths.to_seconds(), 31_536_000);
        assert_eq!(LockDuration::Unlimited.to_seconds(), u64::MAX);
    }

    #[test]
    fn fee_mode_thresholds() {
        assert_eq!(FeeMode::from_daily_count(0), FeeMode::Normal);
        assert_eq!(FeeMode::from_daily_count(99), FeeMode::Normal);
        assert_eq!(FeeMode::from_daily_count(100), FeeMode::Bot);
        assert_eq!(FeeMode::from_daily_count(1000), FeeMode::Bot);
    }

    #[test]
    fn fee_bps() {
        assert_eq!(FeeMode::Normal.fee_bps(), 30);
        assert_eq!(FeeMode::Bot.fee_bps(), 15);
    }

    #[test]
    fn nova_decimals_factor() {
        assert_eq!(NOVA_DECIMALS_FACTOR, 1_000_000_000_000_000_000u128);
    }

    #[test]
    fn address_is_32_bytes() {
        let a = Address::from_seed(1);
        assert_eq!(a.0.len(), 32);
        assert_ne!(a, Address::ZERO);
    }

    #[test]
    fn address_from_seed_unique() {
        assert_ne!(Address::from_seed(1), Address::from_seed(2));
        assert_ne!(Address::from_seed(1), Address::ZERO);
    }

    #[test]
    fn address_display_nova_prefix() {
        let a = Address::from_seed(1);
        assert!(a.to_string().starts_with("nova1"), "Nova adresi 'nova1' ile başlamalı");
    }

    #[test]
    fn address_from_pubkey() {
        let pubkey = [42u8; 64];
        let addr = Address::from_pubkey(&pubkey);
        assert_ne!(addr, Address::ZERO);
        // Aynı pubkey → aynı adres (deterministik)
        assert_eq!(addr, Address::from_pubkey(&pubkey));
    }

    #[test]
    fn asset_id_nova_is_zero() {
        assert_eq!(AssetId::NOVA.0, [0u8; 32]);
    }
}
