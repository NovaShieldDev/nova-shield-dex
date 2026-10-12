//! # Nova DEX
//!
//! Nova Shield Chain üzerinde çalışan native merkeziyetsiz borsa.
//!
//! ## Özellikler
//!
//! | Özellik | Nova DEX | Rakipler |
//! |---------|----------|----------|
//! | Pool oluşturma ücreti | $15-20 sabit | $5-200 değişken |
//! | Swap fee | %0.30 sabit (tek tier) | 4 farklı tier |
//! | Likidite kilitleme | Zorunlu, zincir garantili | Opsiyonel veya yok |
//! | Creator fee | %20 otomatik, günlük | Manuel claim |
//! | Bot indirimi | 100+ işlemde %50 | Yok |
//! | Rug koruması | Teknik olarak imkânsız | Yok |
//! | MEV koruması | Zincir seviyesinde | Yok |
//!
//! ## Kullanım
//!
//! ```rust
//! use nova_dex::{
//!     pool::PoolRegistry,
//!     types::{Address, AssetId, LockDuration, NOVA_DECIMALS_FACTOR},
//! };
//!
//! let mut registry = PoolRegistry::new(0);
//!
//! let creator = Address::from_seed(1);
//! let nova    = AssetId::from_name("NOVA");
//! let usdc    = AssetId::from_name("USDC");
//!
//! // Pool oluştur (7 NOVA ücret @ $2.50/NOVA = $17.50)
//! let pool_id = registry.create_pool(
//!     creator,
//!     nova,
//!     usdc,
//!     1_000 * NOVA_DECIMALS_FACTOR,  // initial A
//!     1_000 * NOVA_DECIMALS_FACTOR,  // initial B
//!     LockDuration::OneWeek,
//!     7 * NOVA_DECIMALS_FACTOR,      // creation fee
//!     250,                           // NOVA price: $2.50
//!     0,                             // genesis time
//! ).expect("pool oluşturma");
//!
//! // Swap yap
//! let swap_result = registry.swap(
//!     Address::from_seed(2), // trader
//!     pool_id,
//!     true,                             // asset_a → asset_b
//!     10 * NOVA_DECIMALS_FACTOR,        // 10 NOVA giriş
//!     0,                                // slippage koruması yok
//!     1_000,                            // zaman
//! ).expect("swap başarılı");
//!
//! println!("Çıkış: {} nNOVA", swap_result.amount_out);
//! println!("Fiyat etkisi: {} bps", swap_result.price_impact_bps);
//! ```

// ─── MODÜLLER ────────────────────────────────────────────────────────────────

/// AMM çekirdeği: x · y = k formülleri.
pub mod amm;

/// Bot sayaç sistemi: günlük işlem takibi ve fee modu belirleme.
pub mod bot_counter;

/// Creator ödeme motoru: 24 saatlik toplu ödemeler.
pub mod creator_payout;

/// Hata tipleri.
pub mod error;

/// Likidite kilitleme mekanizması: rug pull koruması.
pub mod liquidity;

/// MEV koruması: commit-reveal şeması.
pub mod mev;

/// Pool factory ve kayıt defteri: ana DEX koordinatörü.
pub mod pool;

/// Temel veri tipleri ve protokol sabitleri.
pub mod types;

/// Kayıt defteri anlık görüntüsü — RocksDB kalıcılığı için.
#[cfg(feature = "serde")]
pub mod snapshot;

// ─── RE-EXPORT ───────────────────────────────────────────────────────────────

pub use error::DexError;
pub use pool::{Pool, PoolRegistry};
pub use types::{
    Address, Amount, AssetId, FeeMode, LockDuration, PoolId, Timestamp,
    BOT_TX_THRESHOLD, CREATOR_FEE_SHARE_DENOMINATOR, CREATOR_FEE_SHARE_NUMERATOR,
    DEFAULT_SLIPPAGE_BPS, NOVA_DECIMALS_FACTOR, PAYOUT_CYCLE_SECS,
    PAYOUT_TRANSFER_FEE_NNOVA, POOL_CREATION_FEE_USD_CENTS,
    TREASURY_FEE_SHARE_DENOMINATOR, TREASURY_FEE_SHARE_NUMERATOR, UTC_DAY_SECS,
};
