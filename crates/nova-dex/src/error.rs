//! Nova DEX — Hata tipleri.
//!
//! Tüm DEX işlemlerinin dönebileceği hatalar tek bir enum altında toplanmıştır.
//! Zincir seviyesinde reddedilen işlemler bu hataları döner.

use thiserror::Error;

/// Nova DEX işlem hataları.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DexError {
    // ─── Genel ────────────────────────────────────────────────────────────────

    #[error("Sıfır miktar geçersizdir")]
    ZeroAmount,

    #[error("Rezerv sıfır olamaz")]
    ZeroReserve,

    #[error("Aritmetik taşma")]
    Overflow,

    #[error("NOVA fiyatı sıfır olamaz")]
    ZeroNovaPrice,

    #[error("Yetkisiz: bu işlem yalnızca pool creator tarafından yapılabilir")]
    Unauthorized,

    // ─── Pool ─────────────────────────────────────────────────────────────────

    #[error("Pool zaten mevcut")]
    PoolAlreadyExists,

    #[error("Pool bulunamadı")]
    PoolNotFound,

    #[error("Geçersiz token çifti: token_a ve token_b farklı olmalıdır")]
    InvalidTokenPair,

    #[error(
        "Yetersiz pool oluşturma ücreti: gerekli {required} nNOVA, ödenen {provided} nNOVA"
    )]
    InsufficientCreationFee { required: u128, provided: u128 },

    // ─── Swap ─────────────────────────────────────────────────────────────────

    #[error(
        "Yetersiz likidite: reserve_in={reserve_in}, reserve_out={reserve_out}"
    )]
    InsufficientLiquidity { reserve_in: u128, reserve_out: u128 },

    #[error(
        "Slippage toleransı aşıldı: minimum kabul edilebilir {min_amount_out} nNOVA, hesaplanan {actual_amount_out} nNOVA"
    )]
    SlippageExceeded {
        min_amount_out: u128,
        actual_amount_out: u128,
    },

    // ─── Likidite Kilitleme ───────────────────────────────────────────────────

    #[error(
        "Likidite kilitli: kilit sona erme zamanı {unlock_at}, şu anki zaman {current_time}"
    )]
    LiquidityLocked { unlock_at: u64, current_time: u64 },

    #[error(
        "Kilit süresi kısaltılamaz: mevcut kalan {current_secs} saniye, girilen {attempted_secs} saniye"
    )]
    CannotShortenLock { current_secs: u64, attempted_secs: u64 },

    #[error("Sınırsız kilit değiştirilemez")]
    UnlimitedLockImmutable,

    #[error(
        "Yetersiz likidite bakiyesi: mevcut {available} LP token, talep edilen {requested} LP token"
    )]
    InsufficientBalance { available: u128, requested: u128 },

    #[error("Likidite sağlayıcı bulunamadı")]
    LiquidityProviderNotFound,

    // ─── MEV Koruması ─────────────────────────────────────────────────────────

    #[error("Bu işlem için commit zaten mevcut")]
    CommitAlreadyExists,

    #[error("Mempool dolu: maksimum commit limitine ulaşıldı")]
    MempoolFull,

    #[error("Commit bulunamadı")]
    CommitNotFound,

    #[error("Commit zaten açıklandı")]
    AlreadyRevealed,

    #[error("Reveal hash commit ile eşleşmiyor")]
    RevealMismatch,

    #[error("Slot henüz kapanmadı, reveal için beklenmeli")]
    SlotStillOpen,

    // ─── Creator Ödeme ────────────────────────────────────────────────────────

    #[error("Ödeme başarısız: {reason}")]
    PayoutFailed { reason: String },

    // ─── Snapshot ─────────────────────────────────────────────────────────────

    #[error(
        "Snapshot versiyon uyumsuzluğu: beklenen {expected}, bulunan {found}"
    )]
    SnapshotVersionMismatch { expected: u32, found: u32 },
}
