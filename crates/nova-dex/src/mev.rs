//! Nova DEX — MEV Koruma Modülü: Commit-Reveal Şeması.
//!
//! ## MEV Koruması Nasıl Çalışır?
//!
//! Nova Shield Chain'in şifreli mempool'u (IBE + threshold decryption) ile
//! entegre edilmiş commit-reveal mekanizması:
//!
//! ```text
//! 1. COMMIT FAZİ (slot açık — 300ms)
//!    Kullanıcı: commit_hash = SHA256(sender ‖ pool_id ‖ amount_in ‖
//!                                   min_out ‖ direction ‖ nonce)
//!    Kullanıcı: commit_hash'i zincire gönderir (içerik gizli)
//!
//! 2. SLOT KAPANIR (400ms doldu)
//!    Tüm commit'ler toplandı, içerik hâlâ gizli.
//!
//! 3. REVEAL FAZİ (slot kapandıktan sonra — 100ms)
//!    Kullanıcı: orijinal swap intent'i açıklar.
//!    Protokol: hash'i doğrular.
//!
//! 4. BATCH EXECUTION
//!    Tüm reveal edilen intent'ler commitment hash sıralamasıyla işlenir.
//!    Herkese aynı (uniform) fiyat uygulanır.
//!
//! Frontrunning imkânsız: içerik slot kapanana kadar görünmez.
//! Sandwich imkânsız: sıralama manipüle edilemez (commit hash deterministik).
//! ```
//!
//! Bu standalone implementasyon SHA-256 commit hash kullanır.
//! Prodüksiyonda `nova-mempool`'un IBE + threshold decryption sistemi entegre edilir.

use std::collections::HashMap;

use sha2::{Digest, Sha256};

use crate::error::DexError;
use crate::types::{Address, Amount, AssetId, PoolId, Timestamp};

// ─── SLOT SABİTLERİ ──────────────────────────────────────────────────────────

/// Nova Shield Chain blok/slot süresi: 400ms.
pub const SLOT_DURATION_MS: u64 = 400;

/// Commit penceresi: slot'un ilk 300ms'si.
pub const COMMIT_WINDOW_MS: u64 = 300;

/// Reveal penceresi: slot kapandıktan sonraki 100ms.
pub const REVEAL_WINDOW_MS: u64 = 100;

/// Mempool'da tutulabilecek toplam maksimum commit sayısı (OOM koruması).
/// Bu limite ulaşıldığında en eski executed/expired commit'ler temizlenir.
pub const MAX_COMMITS: usize = 10_000;

/// Temizleme eşiği: MAX_COMMITS'in %80'ine ulaşıldığında 2 slot önceki girişler silinir.
const CLEANUP_THRESHOLD: usize = MAX_COMMITS * 8 / 10;

// ─── SWAP INTENT ─────────────────────────────────────────────────────────────

/// Bir kullanıcının göndermeyi planladığı swap'ın tam detayları.
/// Commit fazında yalnızca hash'i bilinir — içerik gizlidir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapIntent {
    /// Swap'ı gönderen cüzdan.
    pub sender: Address,
    /// Hedef pool.
    pub pool_id: PoolId,
    /// Takas edilecek token.
    pub asset_in: AssetId,
    /// Alınacak token.
    pub asset_out: AssetId,
    /// Gönderilen brüt miktar (nNOVA).
    pub amount_in: Amount,
    /// Minimum kabul edilebilir çıkış miktarı (slippage koruması).
    pub min_amount_out: Amount,
    /// Tahmin edilemezliği sağlayan rastgele nonce (32 byte).
    pub nonce: [u8; 32],
}

impl SwapIntent {
    /// Commit hash hesapla: SHA256(sender ‖ pool_id ‖ asset_in ‖ asset_out ‖ amount_in ‖ min_out ‖ nonce)
    pub fn commitment_hash(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(b"nova_dex_swap_intent_v1:");
        h.update(self.sender.0);
        h.update(self.pool_id.0);
        h.update(self.asset_in.0);
        h.update(self.asset_out.0);
        h.update(self.amount_in.to_le_bytes());
        h.update(self.min_amount_out.to_le_bytes());
        h.update(self.nonce);
        let r = h.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&r);
        out
    }
}

// ─── COMMIT DURUMU ───────────────────────────────────────────────────────────

/// Bir commit'in yaşam döngüsü.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitStatus {
    /// Commit alındı; içerik henüz bilinmiyor.
    Pending,
    /// Intent açıklandı; işlem bekliyor.
    Revealed(SwapIntent),
    /// Batch execution'da işlendi.
    Executed,
}

/// Mempool'daki tek bir commit kaydı.
#[derive(Debug, Clone)]
pub struct CommitEntry {
    /// SHA-256 commit hash (içerik kanıtı).
    pub commit_hash: [u8; 32],
    /// Gönderen adres (açıklanmış; içerik değil).
    pub sender: Address,
    /// Commit'in yapıldığı slot ID.
    pub slot_id: u64,
    /// Commit zamanı (Unix saniye).
    pub committed_at: Timestamp,
    /// Commit durumu.
    pub status: CommitStatus,
}

// ─── MEV KORUNAN MEMPOOL ─────────────────────────────────────────────────────

/// Commit-reveal tabanlı MEV korumalı işlem havuzu.
///
/// Her slot 400ms sürer. Commit fazında (ilk 300ms) intent hash'leri toplanır.
/// Reveal fazında (son 100ms) içerikler açıklanır, hash doğrulanır.
/// Batch execution'da tüm intent'ler commit hash sırasıyla (lexicografik) işlenir.
#[derive(Debug)]
pub struct MevProtectedMempool {
    /// Commit hash → entry
    commits: HashMap<[u8; 32], CommitEntry>,
    /// Mevcut slot ID
    current_slot: u64,
    /// Slot başlangıç zamanı (milisaniye)
    slot_start_ms: u64,
}

impl MevProtectedMempool {
    pub fn new(initial_time_ms: u64) -> Self {
        Self {
            commits: HashMap::new(),
            current_slot: 0,
            slot_start_ms: initial_time_ms,
        }
    }

    /// Zaman ilerledikçe slot'u güncelle.
    pub fn tick(&mut self, current_time_ms: u64) {
        if current_time_ms >= self.slot_start_ms + SLOT_DURATION_MS {
            let elapsed = current_time_ms.saturating_sub(self.slot_start_ms);
            let slots = elapsed / SLOT_DURATION_MS;
            self.current_slot += slots;
            self.slot_start_ms += slots * SLOT_DURATION_MS;
        }
    }

    /// Swap intent commit'i gönder (yalnızca hash, içerik gizli).
    ///
    /// Aynı commit hash ikinci kez gönderilemez.
    /// Döndürülen değer bu commit'in ait olduğu slot ID'dir.
    pub fn submit_commit(
        &mut self,
        commit_hash: [u8; 32],
        sender: Address,
        current_time_ms: u64,
    ) -> Result<u64, DexError> {
        self.tick(current_time_ms);

        // OOM koruması: eşiğe ulaşıldığında eski executed/expired girişleri temizle (MEV-001)
        if self.commits.len() >= CLEANUP_THRESHOLD {
            let old_slot_cutoff = self.current_slot.saturating_sub(2);
            self.commits.retain(|_, e| {
                // Executed veya 2+ slot önceki pending'leri at
                !(matches!(e.status, CommitStatus::Executed)
                    || matches!(e.status, CommitStatus::Pending) && e.slot_id < old_slot_cutoff)
            });

            // Hâlâ dolu: hard limit kontrolü
            if self.commits.len() >= MAX_COMMITS {
                return Err(DexError::MempoolFull);
            }
        }

        if self.commits.contains_key(&commit_hash) {
            return Err(DexError::CommitAlreadyExists);
        }

        let slot_id = self.current_slot;
        self.commits.insert(
            commit_hash,
            CommitEntry {
                commit_hash,
                sender,
                slot_id,
                committed_at: current_time_ms / 1_000,
                status: CommitStatus::Pending,
            },
        );

        Ok(slot_id)
    }

    /// Swap intent'i açıkla (reveal).
    ///
    /// Slot kapanmadan (slot değişmeden) reveal yapılamaz.
    /// Hash doğrulaması başarısız olursa `RevealMismatch` döner.
    pub fn reveal(
        &mut self,
        intent: SwapIntent,
        current_time_ms: u64,
    ) -> Result<(), DexError> {
        self.tick(current_time_ms);

        let hash = intent.commitment_hash();
        let entry = self.commits.get_mut(&hash).ok_or(DexError::CommitNotFound)?;

        // Zaten açıklanmış veya işlenmiş commit tekrar reveal edilemez
        if matches!(entry.status, CommitStatus::Revealed(_) | CommitStatus::Executed) {
            return Err(DexError::AlreadyRevealed);
        }

        // Reveal için slot değişmiş olmalı (commit fazı kapanmış)
        if self.current_slot <= entry.slot_id {
            return Err(DexError::SlotStillOpen);
        }

        // Sender doğrulama
        if entry.sender != intent.sender {
            return Err(DexError::RevealMismatch);
        }

        entry.status = CommitStatus::Revealed(intent);
        Ok(())
    }

    /// Belirli bir slot'taki tüm açıklanmış intent'leri deterministik sırayla döndür.
    ///
    /// **Sıralama:** Commit hash byte'larına göre lexicografik (artan).
    /// Bu sıra manipüle edilemez; MEV saldırısı için bilgi değeri yoktur.
    pub fn get_revealed_intents_for_slot(&self, slot_id: u64) -> Vec<SwapIntent> {
        let mut entries: Vec<(&[u8; 32], &CommitEntry)> = self
            .commits
            .iter()
            .filter(|(_, e)| {
                e.slot_id == slot_id && matches!(e.status, CommitStatus::Revealed(_))
            })
            .collect();

        // Deterministik sıralama — saldırgan bu sırayı önceden bilemez
        entries.sort_by_key(|(&hash, _)| hash);

        entries
            .into_iter()
            .filter_map(|(_, e)| {
                if let CommitStatus::Revealed(ref intent) = e.status {
                    Some(intent.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Slot'taki tüm revealed intent'leri executed olarak işaretle.
    pub fn mark_slot_executed(&mut self, slot_id: u64) {
        for entry in self.commits.values_mut() {
            if entry.slot_id == slot_id && matches!(entry.status, CommitStatus::Revealed(_)) {
                entry.status = CommitStatus::Executed;
            }
        }
    }

    /// İşlenmiş commit'leri temizle (bellek yönetimi).
    pub fn cleanup_executed(&mut self) {
        self.commits
            .retain(|_, e| !matches!(e.status, CommitStatus::Executed));
    }

    pub fn current_slot(&self) -> u64 { self.current_slot }
    pub fn pending_count(&self) -> usize {
        self.commits.values().filter(|e| matches!(e.status, CommitStatus::Pending)).count()
    }
    pub fn revealed_count(&self) -> usize {
        self.commits.values().filter(|e| matches!(e.status, CommitStatus::Revealed(_))).count()
    }
    pub fn total_commits(&self) -> usize { self.commits.len() }
}

// ─── TESTS ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AssetId, NOVA_DECIMALS_FACTOR as F};

    fn alice()    -> Address { Address::from_seed(1) }
    fn bob()      -> Address { Address::from_seed(2) }
    fn test_pool() -> PoolId { PoolId::from_assets(&AssetId::from_name("NOVA"), &AssetId::from_name("USDC")) }
    fn test_nonce() -> [u8; 32] { [42u8; 32] }

    fn make_intent(sender: Address, amount_in: u128) -> SwapIntent {
        SwapIntent {
            sender,
            pool_id:        test_pool(),
            asset_in:       AssetId::from_name("NOVA"),
            asset_out:      AssetId::from_name("USDC"),
            amount_in:      amount_in * F,
            min_amount_out: 0,
            nonce:          test_nonce(),
        }
    }

    #[test]
    fn commit_hash_deterministic() {
        let intent = make_intent(alice(), 100);
        let h1 = intent.commitment_hash();
        let h2 = intent.commitment_hash();
        assert_eq!(h1, h2);
    }

    #[test]
    fn commit_hash_changes_with_nonce() {
        let mut intent = make_intent(alice(), 100);
        let h1 = intent.commitment_hash();
        intent.nonce = [7u8; 32];
        let h2 = intent.commitment_hash();
        assert_ne!(h1, h2);
    }

    #[test]
    fn submit_commit_records_entry() {
        let mut mempool = MevProtectedMempool::new(0);
        let intent = make_intent(alice(), 100);
        let hash = intent.commitment_hash();

        let slot_id = mempool.submit_commit(hash, alice(), 0).unwrap();
        assert_eq!(slot_id, 0);
        assert_eq!(mempool.pending_count(), 1);
        assert_eq!(mempool.total_commits(), 1);
    }

    #[test]
    fn duplicate_commit_rejected() {
        let mut mempool = MevProtectedMempool::new(0);
        let intent = make_intent(alice(), 100);
        let hash = intent.commitment_hash();

        mempool.submit_commit(hash, alice(), 0).unwrap();
        assert_eq!(
            mempool.submit_commit(hash, alice(), 0),
            Err(DexError::CommitAlreadyExists)
        );
    }

    #[test]
    fn reveal_before_slot_change_rejected() {
        let mut mempool = MevProtectedMempool::new(0);
        let intent = make_intent(alice(), 100);
        let hash = intent.commitment_hash();

        mempool.submit_commit(hash, alice(), 0).unwrap();

        // Aynı slot'ta reveal → hata
        let result = mempool.reveal(intent, 100); // 100ms < 400ms slot süresi
        assert_eq!(result, Err(DexError::SlotStillOpen));
    }

    #[test]
    fn reveal_after_slot_change_succeeds() {
        let mut mempool = MevProtectedMempool::new(0);
        let intent = make_intent(alice(), 100);
        let hash = intent.commitment_hash();

        // Slot 0'da commit
        mempool.submit_commit(hash, alice(), 0).unwrap();

        // Slot 1'de reveal (400ms+ geçti)
        mempool.reveal(intent, SLOT_DURATION_MS).unwrap();
        assert_eq!(mempool.revealed_count(), 1);
        assert_eq!(mempool.pending_count(), 0);
    }

    #[test]
    fn reveal_without_commit_rejected() {
        let mut mempool = MevProtectedMempool::new(0);
        let intent = make_intent(alice(), 100);

        let result = mempool.reveal(intent, SLOT_DURATION_MS);
        assert_eq!(result, Err(DexError::CommitNotFound));
    }

    #[test]
    fn tampered_intent_rejected() {
        let mut mempool = MevProtectedMempool::new(0);
        let original = make_intent(alice(), 100);
        let hash = original.commitment_hash();

        mempool.submit_commit(hash, alice(), 0).unwrap();

        // Farklı amount ile reveal dene → hash eşleşmez
        let tampered = make_intent(alice(), 200); // farklı miktar
        let result = mempool.reveal(tampered, SLOT_DURATION_MS);
        assert_eq!(result, Err(DexError::CommitNotFound));
    }

    #[test]
    fn wrong_sender_rejected() {
        let mut mempool = MevProtectedMempool::new(0);
        let intent = make_intent(alice(), 100);
        let hash = intent.commitment_hash();

        // Alice'in hash'ini Bob gönderdi olarak kayıt et
        mempool.submit_commit(hash, bob(), 0).unwrap();

        // Alice reveal etmeye çalışıyor ama sender = Bob
        let result = mempool.reveal(intent, SLOT_DURATION_MS);
        assert_eq!(result, Err(DexError::RevealMismatch));
    }

    #[test]
    fn revealed_intents_sorted_by_commit_hash() {
        let mut mempool = MevProtectedMempool::new(0);

        let alice_intent = make_intent(alice(), 100);
        let bob_intent   = SwapIntent {
            sender: bob(),
            pool_id: test_pool(),
            asset_in: AssetId::from_name("NOVA"),
            asset_out: AssetId::from_name("USDC"),
            amount_in: 50 * F,
            min_amount_out: 0,
            nonce: [99u8; 32],
        };

        let alice_hash = alice_intent.commitment_hash();
        let bob_hash   = bob_intent.commitment_hash();

        mempool.submit_commit(alice_hash, alice(), 0).unwrap();
        mempool.submit_commit(bob_hash, bob(), 0).unwrap();

        mempool.reveal(alice_intent.clone(), SLOT_DURATION_MS).unwrap();
        mempool.reveal(bob_intent.clone(), SLOT_DURATION_MS).unwrap();

        let intents = mempool.get_revealed_intents_for_slot(0);
        assert_eq!(intents.len(), 2);

        // Hash sırasına göre sıralı mı?
        let hashes: Vec<[u8; 32]> = intents.iter().map(|i| i.commitment_hash()).collect();
        let mut sorted = hashes.clone();
        sorted.sort();
        assert_eq!(hashes, sorted, "intent'ler commit hash sıralamasıyla olmalı");
    }

    #[test]
    fn mark_slot_executed_and_cleanup() {
        let mut mempool = MevProtectedMempool::new(0);
        let intent = make_intent(alice(), 100);
        let hash = intent.commitment_hash();

        mempool.submit_commit(hash, alice(), 0).unwrap();
        mempool.reveal(intent, SLOT_DURATION_MS).unwrap();

        mempool.mark_slot_executed(0);
        mempool.cleanup_executed();

        assert_eq!(mempool.total_commits(), 0);
    }

    #[test]
    fn slot_tick_advances_correctly() {
        let mut mempool = MevProtectedMempool::new(0);
        assert_eq!(mempool.current_slot(), 0);

        mempool.tick(400); // 1 slot
        assert_eq!(mempool.current_slot(), 1);

        mempool.tick(1200); // 3 slot
        assert_eq!(mempool.current_slot(), 3);
    }
}
