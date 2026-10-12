//! Nova DEX — Likidite Kilitleme Mekanizması.
//!
//! ## Özellikler
//!
//! - Pool açılırken seçilen süre zincire yazılır, değiştirilemez.
//! - Süre **uzatılabilir**, **kısaltılamaz**.
//! - Süre dolmadan likidite çekme zincir tarafından **kesinlikle reddedilir**.
//! - `Unlimited` seçenler "Permanent Liquidity" rozeti alır — sonsuza kadar kilitli.
//! - Rug pull teknik olarak **imkansız**: harici araç veya governance gerekmez.

use crate::error::DexError;
use crate::types::{Address, Amount, LockDuration, Timestamp};

// ─── LOCK INFO ────────────────────────────────────────────────────────────────

/// Bir pool'a ait likidite kilit bilgisi.
///
/// `locked_at` + `duration.to_seconds()` = `unlock_at`.
/// `Unlimited` için `unlock_at = u64::MAX`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockInfo {
    /// Kiti oluşturan creator.
    pub creator: Address,
    /// Seçilen süre (bir kez seçilir; sonradan yalnızca uzatılabilir).
    pub duration: LockDuration,
    /// Kilidin oluşturulduğu zaman (Unix saniye).
    pub locked_at: Timestamp,
    /// Kilidin açılacağı zaman (Unix saniye; Unlimited → u64::MAX).
    pub unlock_at: Timestamp,
}

impl LockInfo {
    /// Yeni kilit oluştur.
    pub fn new(creator: Address, duration: LockDuration, locked_at: Timestamp) -> Self {
        let unlock_at = if duration.is_unlimited() {
            u64::MAX
        } else {
            locked_at.saturating_add(duration.to_seconds())
        };
        Self { creator, duration, locked_at, unlock_at }
    }

    /// Verilen zaman'da kilit hâlâ aktif mi?
    pub fn is_locked(&self, current_time: Timestamp) -> bool {
        if self.duration.is_unlimited() {
            return true; // Daima kilitli
        }
        current_time < self.unlock_at
    }

    /// Kalan kilit süresi (saniye).
    /// - `0` → Kilit açık
    /// - `u64::MAX` → Unlimited
    pub fn remaining_secs(&self, current_time: Timestamp) -> u64 {
        if self.duration.is_unlimited() {
            return u64::MAX;
        }
        self.unlock_at.saturating_sub(current_time)
    }

    /// Likiditenin çekilmesine izin ver ya da [`DexError::LiquidityLocked`] döndür.
    ///
    /// Bu fonksiyon çağrılmadan hiçbir `remove_liquidity` işlemi tamamlanamaz.
    pub fn assert_withdrawable(&self, current_time: Timestamp) -> Result<(), DexError> {
        if self.is_locked(current_time) {
            Err(DexError::LiquidityLocked {
                unlock_at: self.unlock_at,
                current_time,
            })
        } else {
            Ok(())
        }
    }

    /// Kilit süresini uzat.
    ///
    /// # Kurallar
    /// - `Unlimited` → değiştirilemez (`UnlimitedLockImmutable`)
    /// - `new_duration` mevcut kalan süreden **az** olamaz (`CannotShortenLock`)
    /// - `new_duration == Unlimited` → daima izin verilir
    pub fn extend(&mut self, new_duration: LockDuration, current_time: Timestamp) -> Result<(), DexError> {
        if self.duration.is_unlimited() {
            return Err(DexError::UnlimitedLockImmutable);
        }

        let current_remaining = self.remaining_secs(current_time);
        let new_secs = new_duration.to_seconds();

        if !new_duration.is_unlimited() && new_secs <= current_remaining {
            return Err(DexError::CannotShortenLock {
                current_secs:   current_remaining,
                attempted_secs: new_secs,
            });
        }

        self.duration = new_duration;
        self.unlock_at = if new_duration.is_unlimited() {
            u64::MAX
        } else {
            current_time.saturating_add(new_secs)
        };
        Ok(())
    }
}

// ─── LP POZİSYON ─────────────────────────────────────────────────────────────

/// Bir likidite sağlayıcının pool'daki pozisyonu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LpPosition {
    /// LP sağlayıcının adresi.
    pub provider: Address,
    /// Elinde tuttuğu LP token miktarı.
    pub lp_tokens: Amount,
    /// Toplam yatırılan token_a miktarı.
    pub deposited_a: Amount,
    /// Toplam yatırılan token_b miktarı.
    pub deposited_b: Amount,
    /// Pozisyonun oluşturulduğu zaman.
    pub created_at: Timestamp,
}

impl LpPosition {
    pub fn new(
        provider: Address,
        lp_tokens: Amount,
        deposited_a: Amount,
        deposited_b: Amount,
        created_at: Timestamp,
    ) -> Self {
        Self { provider, lp_tokens, deposited_a, deposited_b, created_at }
    }
}

// ─── TESTS ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Address;

    fn creator() -> Address { Address::from_seed(1) }

    #[test]
    fn new_lock_one_day() {
        let info = LockInfo::new(creator(), LockDuration::OneDay, 0);
        assert_eq!(info.unlock_at, 86_400);
        assert!(info.is_locked(0));
        assert!(info.is_locked(86_399));
        assert!(!info.is_locked(86_400)); // tam sınır: artık açık
        assert!(!info.is_locked(100_000));
    }

    #[test]
    fn new_lock_unlimited_always_locked() {
        let info = LockInfo::new(creator(), LockDuration::Unlimited, 0);
        assert!(info.is_locked(0));
        assert!(info.is_locked(u64::MAX / 2));
        assert_eq!(info.unlock_at, u64::MAX);
        assert_eq!(info.remaining_secs(100_000), u64::MAX);
    }

    #[test]
    fn assert_withdrawable_blocked_when_locked() {
        let info = LockInfo::new(creator(), LockDuration::OneDay, 0);
        assert_eq!(
            info.assert_withdrawable(3_600),
            Err(DexError::LiquidityLocked { unlock_at: 86_400, current_time: 3_600 })
        );
    }

    #[test]
    fn assert_withdrawable_ok_after_lock() {
        let info = LockInfo::new(creator(), LockDuration::OneDay, 0);
        assert!(info.assert_withdrawable(86_400).is_ok());
        assert!(info.assert_withdrawable(200_000).is_ok());
    }

    #[test]
    fn extend_lock_one_day_to_one_week() {
        let mut info = LockInfo::new(creator(), LockDuration::OneDay, 0);
        let current_time = 3_600u64; // 1 saat geçmiş
        info.extend(LockDuration::OneWeek, current_time).expect("uzatma başarılı olmalı");
        assert_eq!(info.duration, LockDuration::OneWeek);
        assert_eq!(info.unlock_at, current_time + 604_800);
    }

    #[test]
    fn extend_to_unlimited_always_ok() {
        let mut info = LockInfo::new(creator(), LockDuration::TwelveMonths, 0);
        info.extend(LockDuration::Unlimited, 0).expect("unlimited'a uzatma daima geçerli");
        assert!(info.duration.is_unlimited());
        assert_eq!(info.unlock_at, u64::MAX);
    }

    #[test]
    fn cannot_shorten_lock() {
        let mut info = LockInfo::new(creator(), LockDuration::OneWeek, 0);
        let err = info.extend(LockDuration::OneDay, 0).unwrap_err();
        assert!(
            matches!(err, DexError::CannotShortenLock { .. }),
            "kısaltma hatası bekleniyor"
        );
    }

    #[test]
    fn cannot_modify_unlimited_lock() {
        let mut info = LockInfo::new(creator(), LockDuration::Unlimited, 0);
        assert_eq!(
            info.extend(LockDuration::OneDay, 0),
            Err(DexError::UnlimitedLockImmutable)
        );
    }

    #[test]
    fn remaining_secs_decreases_with_time() {
        let info = LockInfo::new(creator(), LockDuration::OneDay, 0);
        let r0 = info.remaining_secs(0);
        let r1 = info.remaining_secs(3_600);
        assert_eq!(r0, 86_400);
        assert_eq!(r1, 82_800); // 86400 - 3600
    }

    #[test]
    fn rug_pull_impossible() {
        // Creator kilit açılmadan çekmeye çalışıyor — protokol reddeder
        let info = LockInfo::new(creator(), LockDuration::TwelveMonths, 0);
        let halfway = LockDuration::TwelveMonths.to_seconds() / 2;
        let err = info.assert_withdrawable(halfway).unwrap_err();
        assert!(matches!(err, DexError::LiquidityLocked { .. }));
    }
}
