//! Nova DEX — Bot Sayaç Sistemi.
//!
//! ## İşleyiş
//!
//! Her cüzdan için günlük işlem sayısı takip edilir:
//! - 00:00 UTC'de sayaç otomatik sıfırlanır.
//! - 1–99 işlem → [`FeeMode::Normal`] (%0.30 fee)
//! - 100. işlemden itibaren → [`FeeMode::Bot`] (%0.15 fee)
//!
//! Bot indirimi arbitraj ve sniper botları için bilinçli bir teşviktir;
//! hiçbir rakip DEX'te bu özellik bulunmamaktadır.

use std::collections::HashMap;

use crate::types::{Address, FeeMode, Timestamp, UTC_DAY_SECS};

// ─── BOT COUNTER ─────────────────────────────────────────────────────────────

/// Cüzdan başına günlük işlem sayacı.
#[derive(Debug, Default)]
pub struct BotCounter {
    /// Adres → (işlem sayısı, gün başlangıcı timestamp)
    counters: HashMap<Address, (u64, Timestamp)>,
}

impl BotCounter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Verilen timestamp için UTC gün başlangıcını hesapla.
    ///
    /// `day_start = (ts / 86_400) * 86_400`
    fn day_start(ts: Timestamp) -> Timestamp {
        (ts / UTC_DAY_SECS) * UTC_DAY_SECS
    }

    /// `wallet` için `current_time`'daki işlem sayısını artır.
    ///
    /// Yeni bir gün başlamışsa sayaç önce sıfırlanır.
    /// Güncellenmiş sayacı (bu işlem dahil) döner.
    pub fn record_transaction(&mut self, wallet: Address, current_time: Timestamp) -> u64 {
        let today = Self::day_start(current_time);
        let entry = self.counters.entry(wallet).or_insert((0, today));

        // Yeni gün → sıfırla
        if entry.1 < today {
            *entry = (0, today);
        }

        entry.0 += 1;
        entry.0
    }

    /// `wallet`'ın bugünkü işlem sayısını döndür (artırmadan).
    pub fn daily_count(&self, wallet: &Address, current_time: Timestamp) -> u64 {
        let today = Self::day_start(current_time);
        match self.counters.get(wallet) {
            Some(&(count, day)) if day == today => count,
            _ => 0,
        }
    }

    /// Sayacı artırmadan fee modunu önizle.
    ///
    /// Slippage kontrolünden önce kullanılır; başarısız swap'larda sayaç artmaz.
    pub fn preview_fee_mode(&self, wallet: &Address, current_time: Timestamp) -> FeeMode {
        // Bu işlem gerçekleşirse sayaç (mevcut + 1) olacak
        let next_count = self.daily_count(wallet, current_time).saturating_add(1);
        FeeMode::from_daily_count(next_count)
    }

    /// Bir işlem kaydet ve uygulanacak fee modunu döndür.
    ///
    /// Fee, bu işlemin ardından güncellenen sayaca göre belirlenir:
    /// 100. işlem `FeeMode::Bot` ile işlenir.
    pub fn record_and_get_fee_mode(
        &mut self,
        wallet: Address,
        current_time: Timestamp,
    ) -> FeeMode {
        let count = self.record_transaction(wallet, current_time);
        FeeMode::from_daily_count(count)
    }

    /// Tüm sayaçları sıfırla (yalnızca testlerde kullanılır).
    #[cfg(test)]
    pub fn reset_all(&mut self) {
        self.counters.clear();
    }

    // ─── Snapshot ────────────────────────────────────────────────────────────

    /// Snapshot için iç durumu dışa aktar: (adres, günlük_sayı, gün_başlangıcı).
    #[cfg(feature = "serde")]
    pub fn snapshot_entries(&self) -> Vec<([u8; 32], u64, u64)> {
        self.counters
            .iter()
            .map(|(addr, &(count, day))| (addr.0, count, day))
            .collect()
    }

    /// Daha önce alınan snapshot'tan geri yükle.
    #[cfg(feature = "serde")]
    pub fn restore(entries: Vec<([u8; 32], u64, u64)>) -> Self {
        let counters = entries
            .into_iter()
            .map(|(bytes, count, day)| (Address(bytes), (count, day)))
            .collect();
        Self { counters }
    }
}

// ─── TESTS ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn alice() -> Address { Address::from_seed(1) }
    fn bob()   -> Address { Address::from_seed(2) }

    const DAY_1: Timestamp = 0;            // gün 1 başlangıcı (Unix epoch)
    const DAY_2: Timestamp = UTC_DAY_SECS; // gün 2 başlangıcı

    #[test]
    fn first_transaction_is_normal_mode() {
        let mut counter = BotCounter::new();
        let mode = counter.record_and_get_fee_mode(alice(), DAY_1);
        assert_eq!(mode, FeeMode::Normal);
    }

    #[test]
    fn ninety_ninth_transaction_is_normal_mode() {
        let mut counter = BotCounter::new();
        for _ in 0..98 {
            counter.record_transaction(alice(), DAY_1);
        }
        let mode = counter.record_and_get_fee_mode(alice(), DAY_1);
        assert_eq!(counter.daily_count(&alice(), DAY_1), 99);
        assert_eq!(mode, FeeMode::Normal);
    }

    #[test]
    fn hundredth_transaction_triggers_bot_mode() {
        let mut counter = BotCounter::new();
        for _ in 0..99 {
            counter.record_transaction(alice(), DAY_1);
        }
        let mode = counter.record_and_get_fee_mode(alice(), DAY_1);
        assert_eq!(counter.daily_count(&alice(), DAY_1), 100);
        assert_eq!(mode, FeeMode::Bot, "100. işlem bot fee modunda olmalı");
    }

    #[test]
    fn counter_resets_at_utc_midnight() {
        let mut counter = BotCounter::new();

        // Gün 1: 150 işlem yap
        for _ in 0..150 {
            counter.record_transaction(alice(), DAY_1 + 3_600); // 01:00 UTC
        }
        assert_eq!(counter.daily_count(&alice(), DAY_1), 150);
        assert_eq!(
            counter.record_and_get_fee_mode(alice(), DAY_1 + 3_600),
            FeeMode::Bot
        );

        // Gün 2: sayaç sıfırlanır
        assert_eq!(counter.daily_count(&alice(), DAY_2), 0);
        let mode = counter.record_and_get_fee_mode(alice(), DAY_2 + 3_600);
        assert_eq!(mode, FeeMode::Normal, "yeni günde sayaç sıfırlanmalı");
        assert_eq!(counter.daily_count(&alice(), DAY_2), 1);
    }

    #[test]
    fn different_wallets_are_independent() {
        let mut counter = BotCounter::new();

        for _ in 0..100 {
            counter.record_transaction(alice(), DAY_1);
        }

        // Bob henüz işlem yapmadı
        assert_eq!(counter.daily_count(&bob(), DAY_1), 0);
        let bob_mode = counter.record_and_get_fee_mode(bob(), DAY_1);
        assert_eq!(bob_mode, FeeMode::Normal);
    }

    #[test]
    fn reset_all_clears_counters() {
        let mut counter = BotCounter::new();
        for _ in 0..50 {
            counter.record_transaction(alice(), DAY_1);
        }
        counter.reset_all();
        assert_eq!(counter.daily_count(&alice(), DAY_1), 0);
    }

    #[test]
    fn bot_mode_persists_after_threshold() {
        let mut counter = BotCounter::new();
        for _ in 0..100 {
            counter.record_transaction(alice(), DAY_1);
        }
        // 101., 200. işlemler hâlâ bot modunda
        let mode_101 = counter.record_and_get_fee_mode(alice(), DAY_1);
        let mode_200 = {
            for _ in 0..98 {
                counter.record_transaction(alice(), DAY_1);
            }
            counter.record_and_get_fee_mode(alice(), DAY_1)
        };
        assert_eq!(mode_101, FeeMode::Bot);
        assert_eq!(mode_200, FeeMode::Bot);
    }

    #[test]
    fn day_boundary_is_utc_midnight() {
        // 23:59:59 → gün 1
        let end_of_day = UTC_DAY_SECS - 1;
        // 00:00:00 → gün 2
        let start_of_day2 = UTC_DAY_SECS;

        let mut counter = BotCounter::new();
        for _ in 0..150 {
            counter.record_transaction(alice(), end_of_day);
        }

        // Yeni güne geçince sıfırlanmalı
        assert_eq!(counter.daily_count(&alice(), start_of_day2), 0);
    }
}
