//! Nova DEX — Creator Ödeme Motoru.
//!
//! ## İşleyiş
//!
//! - Her swap'ın fee'sinin %20'si creator'a ayrılır (biriktirilir).
//! - 24 saatte bir toplu ödeme yapılır — manuel claim gerekmez.
//! - Transfer ücreti creator'ın alacağından düşülür.
//! - Net miktar direkt creator cüzdanına gönderilir.
//! - 24 saat dolmadan `process_payouts` çağrılsa boş döner.

use std::collections::HashMap;

use crate::error::DexError;
use crate::types::{
    Address, Amount, Timestamp, PAYOUT_CYCLE_SECS,
    PAYOUT_TRANSFER_FEE_NNOVA,
};

// ─── PAYOUT KAYIT ────────────────────────────────────────────────────────────

/// Tek bir ödeme kaydı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayoutRecord {
    /// Alıcı creator.
    pub creator: Address,
    /// Transfer öncesi brüt bakiye.
    pub gross_amount: Amount,
    /// Kesilen transfer ücreti.
    pub transfer_fee: Amount,
    /// Creator'ın cüzdanına ulaşan net miktar.
    pub net_amount: Amount,
    /// Ödeme zamanı (Unix saniye).
    pub paid_at: Timestamp,
}

// ─── CREATOR PAYOUT ENGINE ───────────────────────────────────────────────────

/// Creator'ların birikmiş fee'lerini takip eder ve 24 saatlik toplu ödeme yapar.
#[derive(Debug)]
pub struct CreatorPayoutEngine {
    /// Bekleyen fee miktarı: creator adres → nNOVA
    pending: HashMap<Address, Amount>,
    /// Son ödeme zamanı
    last_payout_at: Timestamp,
    /// Tüm ödeme geçmişi
    payout_history: Vec<PayoutRecord>,
}

impl CreatorPayoutEngine {
    /// Yeni motor, `genesis_time` ile başlatılır.
    pub fn new(genesis_time: Timestamp) -> Self {
        Self {
            pending: HashMap::new(),
            last_payout_at: genesis_time,
            payout_history: Vec::new(),
        }
    }

    /// `creator` için `amount` nNOVA biriktir.
    pub fn accrue(&mut self, creator: Address, amount: Amount) {
        *self.pending.entry(creator).or_insert(0) += amount;
    }

    /// 24 saatlik ödeme döngüsü tamamlandı mı?
    pub fn payout_due(&self, current_time: Timestamp) -> bool {
        current_time >= self.last_payout_at.saturating_add(PAYOUT_CYCLE_SECS)
    }

    /// Tüm bekleyen creator ödemelerini işle.
    ///
    /// - 24 saat dolmamışsa boş `Vec` döner.
    /// - Transfer ücreti (`PAYOUT_TRANSFER_FEE_NNOVA`) net miktardan düşülür.
    /// - Transfer ücreti > brüt ise o creator atlanır (küçük tutarlar sonraki döngüye taşınır).
    ///
    /// Dönen `Vec<PayoutRecord>` çağıran tarafından token transferi için kullanılır.
    pub fn process_payouts(
        &mut self,
        current_time: Timestamp,
    ) -> Result<Vec<PayoutRecord>, DexError> {
        if !self.payout_due(current_time) {
            return Ok(vec![]);
        }

        let mut records = Vec::new();
        let mut paid_creators: Vec<Address> = Vec::new();

        for (&creator, &gross) in &self.pending {
            if gross == 0 {
                paid_creators.push(creator);
                continue;
            }

            if gross <= PAYOUT_TRANSFER_FEE_NNOVA {
                // Transfer ücreti brütten büyük/eşit — sonraki döngüye taşı (kayıp yok)
                continue;
            }

            let net = gross - PAYOUT_TRANSFER_FEE_NNOVA;

            records.push(PayoutRecord {
                creator,
                gross_amount:  gross,
                transfer_fee:  PAYOUT_TRANSFER_FEE_NNOVA,
                net_amount:    net,
                paid_at:       current_time,
            });

            paid_creators.push(creator);
        }

        // Ödenen creator'ların pending bakiyelerini temizle
        for creator in &paid_creators {
            self.pending.remove(creator);
        }

        self.last_payout_at = current_time;
        self.payout_history.extend(records.iter().cloned());

        Ok(records)
    }

    /// Bir creator'ın bekleyen fee bakiyesi.
    pub fn pending_for(&self, creator: &Address) -> Amount {
        *self.pending.get(creator).unwrap_or(&0)
    }

    /// Tüm creator'ların toplam bekleyen bakiyesi.
    pub fn total_pending(&self) -> Amount {
        self.pending.values().copied().sum()
    }

    /// Tüm ödeme geçmişi.
    pub fn payout_history(&self) -> &[PayoutRecord] {
        &self.payout_history
    }

    pub fn last_payout_at(&self) -> Timestamp {
        self.last_payout_at
    }

    // ─── Snapshot ────────────────────────────────────────────────────────────

    /// Bekleyen ödeme girişlerini dışa aktar: (creator_bytes, miktar).
    #[cfg(feature = "serde")]
    pub fn pending_entries(&self) -> Vec<([u8; 32], Amount)> {
        self.pending
            .iter()
            .map(|(addr, &amt)| (addr.0, amt))
            .collect()
    }

    /// Snapshot'tan geri yükle.
    #[cfg(feature = "serde")]
    pub fn restore(
        last_payout_at: Timestamp,
        entries: Vec<([u8; 32], Amount)>,
    ) -> Self {
        let pending = entries
            .into_iter()
            .map(|(bytes, amt)| (Address(bytes), amt))
            .collect();
        Self {
            pending,
            last_payout_at,
            payout_history: Vec::new(),
        }
    }
}

// ─── TESTS ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NOVA_DECIMALS_FACTOR as F;

    fn nova(n: u128) -> Amount { n * F }

    fn alice() -> Address { Address::from_seed(1) }
    fn bob()   -> Address { Address::from_seed(2) }

    #[test]
    fn payout_not_due_before_24h() {
        let mut engine = CreatorPayoutEngine::new(0);
        engine.accrue(alice(), nova(10));

        // 23 saat geçti — ödeme zamanı değil
        let records = engine.process_payouts(23 * 3_600).unwrap();
        assert!(records.is_empty());
        assert_eq!(engine.pending_for(&alice()), nova(10));
    }

    #[test]
    fn payout_executes_exactly_at_24h() {
        let mut engine = CreatorPayoutEngine::new(0);
        engine.accrue(alice(), nova(10));

        let records = engine.process_payouts(PAYOUT_CYCLE_SECS).unwrap();
        assert_eq!(records.len(), 1);

        let rec = &records[0];
        assert_eq!(rec.creator, alice());
        assert_eq!(rec.gross_amount, nova(10));
        assert_eq!(rec.transfer_fee, PAYOUT_TRANSFER_FEE_NNOVA);
        assert_eq!(rec.net_amount, nova(10) - PAYOUT_TRANSFER_FEE_NNOVA);
        assert_eq!(rec.paid_at, PAYOUT_CYCLE_SECS);
    }

    #[test]
    fn pending_cleared_after_payout() {
        let mut engine = CreatorPayoutEngine::new(0);
        engine.accrue(alice(), nova(5));
        engine.process_payouts(PAYOUT_CYCLE_SECS).unwrap();
        assert_eq!(engine.pending_for(&alice()), 0);
    }

    #[test]
    fn small_amount_not_paid_carried_over() {
        let mut engine = CreatorPayoutEngine::new(0);
        // Transfer ücretinden az → atlanır, sonraki döngüye taşınır
        engine.accrue(alice(), PAYOUT_TRANSFER_FEE_NNOVA); // eşit → atlanır
        let records = engine.process_payouts(PAYOUT_CYCLE_SECS).unwrap();
        assert!(records.is_empty(), "transfer ücreti brüt'e eşit → ödeme yapılmaz");
        // Birikmiş bakiye hâlâ orada
        assert_eq!(engine.pending_for(&alice()), PAYOUT_TRANSFER_FEE_NNOVA);
    }

    #[test]
    fn multiple_creators_batched() {
        let mut engine = CreatorPayoutEngine::new(0);
        engine.accrue(alice(), nova(10));
        engine.accrue(bob(), nova(20));

        let records = engine.process_payouts(PAYOUT_CYCLE_SECS).unwrap();
        assert_eq!(records.len(), 2);

        // Her iki creator da ödenmiş
        assert_eq!(engine.pending_for(&alice()), 0);
        assert_eq!(engine.pending_for(&bob()), 0);
    }

    #[test]
    fn accrue_accumulates_correctly() {
        let mut engine = CreatorPayoutEngine::new(0);
        engine.accrue(alice(), nova(3));
        engine.accrue(alice(), nova(7));
        assert_eq!(engine.pending_for(&alice()), nova(10));
    }

    #[test]
    fn payout_history_preserved() {
        let mut engine = CreatorPayoutEngine::new(0);
        engine.accrue(alice(), nova(10));
        engine.process_payouts(PAYOUT_CYCLE_SECS).unwrap();

        engine.accrue(alice(), nova(5));
        engine.process_payouts(PAYOUT_CYCLE_SECS * 2).unwrap();

        assert_eq!(engine.payout_history().len(), 2);
    }

    #[test]
    fn transfer_fee_is_001_nova() {
        // 0.001 NOVA = 10^15 nNOVA
        assert_eq!(PAYOUT_TRANSFER_FEE_NNOVA, 1_000_000_000_000_000u128);
        assert_eq!(PAYOUT_TRANSFER_FEE_NNOVA, F / 1_000);
    }

    #[test]
    fn second_cycle_after_first_payout() {
        let mut engine = CreatorPayoutEngine::new(0);
        engine.accrue(alice(), nova(10));
        engine.process_payouts(PAYOUT_CYCLE_SECS).unwrap();

        // İkinci döngüde yeni birikim yapılıp ödeme alınır
        engine.accrue(alice(), nova(5));
        let records = engine.process_payouts(PAYOUT_CYCLE_SECS * 2).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].gross_amount, nova(5));
    }
}
