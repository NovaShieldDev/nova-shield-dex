//! Nova DEX — Pool Factory ve Kayıt Defteri.
//!
//! `PoolRegistry` tüm DEX modüllerini koordine eden ana yapıdır:
//! - Pool oluşturma (sabit $17.50 ücret)
//! - Likidite ekleme / çıkarma (kilit zorlamalı)
//! - Swap (fee dağıtımı + bot sayacı + MEV koruması)
//! - Creator ödeme motoru (24h toplu)
//! - Pool listeleme (Permanent Liquidity önde)

use std::collections::HashMap;

use crate::amm::{self, SwapComputation};
use crate::bot_counter::BotCounter;
use crate::creator_payout::{CreatorPayoutEngine, PayoutRecord};
use crate::error::DexError;
use crate::liquidity::{LockInfo, LpPosition};
use crate::types::{
    Address, Amount, AssetId, LockDuration, PoolId, Timestamp,
    NOVA_DECIMALS_FACTOR, POOL_CREATION_FEE_USD_CENTS,
};

// ─── POOL OLUŞTURMA ÜCRETİ ───────────────────────────────────────────────────

/// Pool oluşturma için gereken minimum nNOVA miktarını hesapla.
///
/// Hedef: $17.50 değerinde NOVA.
///
/// `nova_price_cents`: 1 NOVA'nın USD cent cinsinden fiyatı (örn. $2.50 → 250).
///
/// `fee_nnova = 1750 × NOVA_DECIMALS_FACTOR / nova_price_cents`
pub fn required_creation_fee_nnova(nova_price_cents: u64) -> Result<Amount, DexError> {
    if nova_price_cents == 0 {
        return Err(DexError::ZeroNovaPrice);
    }
    let fee = (POOL_CREATION_FEE_USD_CENTS as u128)
        .checked_mul(NOVA_DECIMALS_FACTOR)
        .ok_or(DexError::Overflow)?
        .checked_div(nova_price_cents as u128)
        .ok_or(DexError::Overflow)?;
    Ok(fee)
}

// ─── POOL YAPISI ─────────────────────────────────────────────────────────────

/// Bir likidite pool'unun tam zincir durumu.
#[derive(Debug, Clone)]
pub struct Pool {
    pub id: PoolId,
    /// Token çifti
    pub asset_a: AssetId,
    pub asset_b: AssetId,
    /// Pool'u oluşturan adres
    pub creator: Address,
    /// Mevcut rezervler
    pub reserve_a: Amount,
    pub reserve_b: Amount,
    /// Toplam dolaşımdaki LP token miktarı
    pub total_lp: Amount,
    /// Likidite kilit bilgisi
    pub lock_info: LockInfo,
    /// Pool oluşturma zamanı
    pub created_at: Timestamp,
    /// Tüm zamanlarda toplam işlem hacmi (asset_a cinsinden)
    pub total_volume_a: Amount,
    /// Toplam toplanan fee (nNOVA)
    pub total_fees_collected: Amount,
    /// Toplam swap sayısı
    pub swap_count: u64,
}

impl Pool {
    /// Pool "Permanent Liquidity" rozetine sahip mi?
    pub fn is_permanent(&self) -> bool {
        self.lock_info.duration.is_unlimited()
    }

    /// Kilit durumu: kalan süre veya "Permanent Liquidity"
    pub fn lock_status(&self, current_time: Timestamp) -> String {
        if self.lock_info.duration.is_unlimited() {
            return "Permanent Liquidity ♾".to_string();
        }
        let remaining = self.lock_info.remaining_secs(current_time);
        if remaining == 0 {
            "Kilit Açık".to_string()
        } else {
            let days  = remaining / 86_400;
            let hours = (remaining % 86_400) / 3_600;
            let mins  = (remaining % 3_600) / 60;
            format!("Kilitli — {days}g {hours}s {mins}dk kaldı")
        }
    }
}

// ─── POOL REGISTRY ───────────────────────────────────────────────────────────

/// Nova DEX ana kayıt defteri: tüm pool'ları ve alt sistemleri yönetir.
#[derive(Debug)]
pub struct PoolRegistry {
    /// Tüm pool'lar: PoolId → Pool
    pools: HashMap<PoolId, Pool>,
    /// LP pozisyonları: (PoolId, provider Address) → LpPosition
    lp_positions: HashMap<(PoolId, Address), LpPosition>,
    /// Bot sayacı
    bot_counter: BotCounter,
    /// Creator ödeme motoru
    payout_engine: CreatorPayoutEngine,
    /// Kasa (treasury) birikmiş bakiye
    treasury_balance: Amount,
    /// İlk başlatma zamanı (Unix saniye) — snapshot'ta korunur
    genesis_time: Timestamp,
}

impl PoolRegistry {
    /// Yeni kayıt defteri oluştur.
    pub fn new(genesis_time: Timestamp) -> Self {
        Self {
            pools: HashMap::new(),
            lp_positions: HashMap::new(),
            bot_counter: BotCounter::new(),
            payout_engine: CreatorPayoutEngine::new(genesis_time),
            treasury_balance: 0,
            genesis_time,
        }
    }

    // ─── POOL OLUŞTURMA ──────────────────────────────────────────────────────

    /// Yeni bir likidite pool'u oluştur.
    ///
    /// # Adımlar (spec §2, §3)
    /// 1. Token çifti farklı olmalı
    /// 2. Başlangıç likiditesi sıfır olmamalı
    /// 3. Pool oluşturma ücreti ($17.50 hedefli) kontrol edilir
    /// 4. Pool ID türetilir (deterministic, order-independent)
    /// 5. Aynı token çifti için ikinci pool açılamaz
    /// 6. Kilitleme süresi zincire yazılır — değiştirilemez
    /// 7. Creator'ın LP pozisyonu oluşturulur
    ///
    /// # Parametreler
    /// * `creator`             — Pool sahibi
    /// * `asset_a`, `asset_b`  — Token çifti
    /// * `initial_a`, `initial_b` — Başlangıç likiditesi
    /// * `lock_duration`       — Likidite kilitleme süresi (sonradan uzatılabilir, kısaltılamaz)
    /// * `creation_fee_paid`   — Ödenen NOVA (nNOVA cinsinden)
    /// * `nova_price_cents`    — Anlık NOVA fiyatı (USD cent)
    /// * `now`                 — Şu anki zaman (Unix saniye)
    #[allow(clippy::too_many_arguments)]
    pub fn create_pool(
        &mut self,
        creator: Address,
        asset_a: AssetId,
        asset_b: AssetId,
        initial_a: Amount,
        initial_b: Amount,
        lock_duration: LockDuration,
        creation_fee_paid: Amount,
        nova_price_cents: u64,
        now: Timestamp,
    ) -> Result<PoolId, DexError> {
        if asset_a == asset_b {
            return Err(DexError::InvalidTokenPair);
        }
        if initial_a == 0 || initial_b == 0 {
            return Err(DexError::ZeroAmount);
        }

        // Oluşturma ücreti kontrolü
        let required_fee = required_creation_fee_nnova(nova_price_cents)?;
        if creation_fee_paid < required_fee {
            return Err(DexError::InsufficientCreationFee {
                required: required_fee,
                provided: creation_fee_paid,
            });
        }

        let pool_id = PoolId::from_assets(&asset_a, &asset_b);

        if self.pools.contains_key(&pool_id) {
            return Err(DexError::PoolAlreadyExists);
        }

        let initial_lp =
            amm::compute_lp_tokens_to_mint(initial_a, initial_b, 0, 0, 0)?;

        let pool = Pool {
            id: pool_id,
            asset_a,
            asset_b,
            creator,
            reserve_a: initial_a,
            reserve_b: initial_b,
            total_lp: initial_lp,
            lock_info: LockInfo::new(creator, lock_duration, now),
            created_at: now,
            total_volume_a: 0,
            total_fees_collected: 0,
            swap_count: 0,
        };

        // Creator'ın ilk LP pozisyonu
        self.lp_positions.insert(
            (pool_id, creator),
            LpPosition::new(creator, initial_lp, initial_a, initial_b, now),
        );

        self.pools.insert(pool_id, pool);
        // Tüm doğrulamalar geçti — oluşturma ücreti treasury'ye aktarılır (POOL-FEE-001)
        self.treasury_balance = self.treasury_balance.saturating_add(creation_fee_paid);
        Ok(pool_id)
    }

    // ─── SWAP ────────────────────────────────────────────────────────────────

    /// Swap işlemi gerçekleştir.
    ///
    /// # Fee Akışı (spec §4, §5)
    /// - Bot sayacı → `FeeMode` belirlenir
    /// - AMM formülü uygulanır
    /// - Fee: %20 → creator biriktirilir, %80 → treasury
    ///
    /// # Parametreler
    /// * `swapper`        — Swap yapan adres
    /// * `pool_id`        — Hedef pool
    /// * `zero_for_one`   — true: asset_a → asset_b, false: asset_b → asset_a
    /// * `amount_in`      — Brüt giriş miktarı
    /// * `min_amount_out` — Minimum kabul edilebilir çıkış; 0 = slippage yok
    /// * `now`            — Şu anki zaman (Unix saniye)
    pub fn swap(
        &mut self,
        swapper: Address,
        pool_id: PoolId,
        zero_for_one: bool,
        amount_in: Amount,
        min_amount_out: Amount,
        now: Timestamp,
    ) -> Result<SwapComputation, DexError> {
        // 1. Rezervleri oku (immutable borrow — bot sayacı henüz artırılmıyor)
        let fee_mode_preview = self.bot_counter.preview_fee_mode(&swapper, now);

        let (reserve_in, reserve_out) = {
            let pool = self.pools.get(&pool_id).ok_or(DexError::PoolNotFound)?;
            if zero_for_one {
                (pool.reserve_a, pool.reserve_b)
            } else {
                (pool.reserve_b, pool.reserve_a)
            }
        };

        // 2. AMM hesabı (preview fee ile)
        let comp_preview = amm::compute_swap(reserve_in, reserve_out, amount_in, fee_mode_preview)?;

        // 3. Slippage kontrolü — BAŞARILI OLMADAN sayaç artırılmaz (SWAP-001 düzeltmesi)
        if min_amount_out > 0 && comp_preview.amount_out < min_amount_out {
            return Err(DexError::SlippageExceeded {
                min_amount_out,
                actual_amount_out: comp_preview.amount_out,
            });
        }

        // 4. Swap geçti: şimdi bot sayacını kaydet
        let fee_mode = self.bot_counter.record_and_get_fee_mode(swapper, now);

        // 5. Fee modu değiştiyse (99→100 sınırı) yeniden hesapla (düşük fee = daha iyi çıkış)
        let comp = if fee_mode != fee_mode_preview {
            amm::compute_swap(reserve_in, reserve_out, amount_in, fee_mode)?
        } else {
            comp_preview
        };

        // 6. Rezervleri güncelle (fee pool'a girmiyor — creator+treasury'ye gidiyor)
        let pool = self.pools.get_mut(&pool_id).ok_or(DexError::PoolNotFound)?;
        if zero_for_one {
            pool.reserve_a = pool.reserve_a
                .checked_add(comp.amount_in_net)
                .ok_or(DexError::Overflow)?;
            pool.reserve_b = pool.reserve_b
                .checked_sub(comp.amount_out)
                .ok_or(DexError::Overflow)?;
            pool.total_volume_a = pool.total_volume_a.saturating_add(amount_in);
        } else {
            pool.reserve_b = pool.reserve_b
                .checked_add(comp.amount_in_net)
                .ok_or(DexError::Overflow)?;
            pool.reserve_a = pool.reserve_a
                .checked_sub(comp.amount_out)
                .ok_or(DexError::Overflow)?;
            pool.total_volume_a = pool.total_volume_a.saturating_add(comp.amount_out);
        }

        pool.total_fees_collected = pool.total_fees_collected.saturating_add(comp.fee_amount);
        pool.swap_count += 1;

        // 5. Fee dağıtımı
        let creator = pool.creator;
        self.payout_engine.accrue(creator, comp.creator_fee());
        self.treasury_balance = self.treasury_balance.saturating_add(comp.treasury_fee());

        Ok(comp)
    }

    // ─── LİKİDİTE ────────────────────────────────────────────────────────────

    /// Pool'a likidite ekle.
    ///
    /// Döner: `(lp_tokens, actual_a, actual_b)` — basılan LP ve gerçekte kullanılan miktarlar.
    /// Mevcut rezerv oranına göre normalize edilir; fazla token iade edilir (POOL-002).
    pub fn add_liquidity(
        &mut self,
        provider: Address,
        pool_id: PoolId,
        amount_a: Amount,
        amount_b: Amount,
        now: Timestamp,
    ) -> Result<(Amount, Amount, Amount), DexError> {
        if amount_a == 0 || amount_b == 0 {
            return Err(DexError::ZeroAmount);
        }

        let pool = self.pools.get_mut(&pool_id).ok_or(DexError::PoolNotFound)?;

        // Mevcut orana normalize et (fazla token iade edilir)
        let (actual_a, actual_b) = amm::normalize_liquidity(
            amount_a,
            amount_b,
            pool.reserve_a,
            pool.reserve_b,
            pool.total_lp,
        )?;

        let lp_tokens = amm::compute_lp_tokens_to_mint(
            actual_a,
            actual_b,
            pool.reserve_a,
            pool.reserve_b,
            pool.total_lp,
        )?;

        pool.reserve_a = pool.reserve_a.saturating_add(actual_a);
        pool.reserve_b = pool.reserve_b.saturating_add(actual_b);
        pool.total_lp  = pool.total_lp.saturating_add(lp_tokens);

        let pos = self
            .lp_positions
            .entry((pool_id, provider))
            .or_insert_with(|| LpPosition::new(provider, 0, 0, 0, now));

        pos.lp_tokens   = pos.lp_tokens.saturating_add(lp_tokens);
        pos.deposited_a = pos.deposited_a.saturating_add(actual_a);
        pos.deposited_b = pos.deposited_b.saturating_add(actual_b);

        Ok((lp_tokens, actual_a, actual_b))
    }

    /// Pool'dan likidite çek.
    ///
    /// Kilit süresi dolmadan çekme işlemi **protokol tarafından reddedilir**
    /// — bu Nova DEX'in rug pull korumasının temelidir.
    pub fn remove_liquidity(
        &mut self,
        provider: Address,
        pool_id: PoolId,
        lp_to_burn: Amount,
        now: Timestamp,
    ) -> Result<(Amount, Amount), DexError> {
        // Kilit kontrolü — ÖNCE yapılır, sonraki işlemlerden önce
        {
            let pool = self.pools.get(&pool_id).ok_or(DexError::PoolNotFound)?;
            pool.lock_info.assert_withdrawable(now)?;
        }

        let pos = self
            .lp_positions
            .get_mut(&(pool_id, provider))
            .ok_or(DexError::LiquidityProviderNotFound)?;

        if pos.lp_tokens < lp_to_burn {
            return Err(DexError::InsufficientBalance {
                available: pos.lp_tokens,
                requested: lp_to_burn,
            });
        }

        let pool = self.pools.get_mut(&pool_id).ok_or(DexError::PoolNotFound)?;

        let (amount_a, amount_b) = amm::compute_withdrawal_amounts(
            lp_to_burn,
            pool.total_lp,
            pool.reserve_a,
            pool.reserve_b,
        )?;

        pool.reserve_a = pool.reserve_a.saturating_sub(amount_a);
        pool.reserve_b = pool.reserve_b.saturating_sub(amount_b);
        pool.total_lp  = pool.total_lp.saturating_sub(lp_to_burn);

        let pos = self.lp_positions.get_mut(&(pool_id, provider)).unwrap();
        pos.lp_tokens -= lp_to_burn;

        Ok((amount_a, amount_b))
    }

    // ─── KİLİT UZATMA ────────────────────────────────────────────────────────

    /// Pool'un kilit süresini uzat.
    ///
    /// Yalnızca creator yapabilir. Kısaltma yasaktır.
    pub fn extend_lock(
        &mut self,
        caller: Address,
        pool_id: PoolId,
        new_duration: LockDuration,
        now: Timestamp,
    ) -> Result<(), DexError> {
        let pool = self.pools.get_mut(&pool_id).ok_or(DexError::PoolNotFound)?;

        if pool.creator != caller {
            return Err(DexError::Unauthorized);
        }

        pool.lock_info.extend(new_duration, now)
    }

    // ─── CREATOR ÖDEMELER ────────────────────────────────────────────────────

    /// Bekleyen creator fee ödemelerini işle (24 saatte bir).
    pub fn process_creator_payouts(
        &mut self,
        now: Timestamp,
    ) -> Result<Vec<PayoutRecord>, DexError> {
        self.payout_engine.process_payouts(now)
    }

    // ─── SORGULAR ────────────────────────────────────────────────────────────

    pub fn get_pool(&self, pool_id: &PoolId) -> Option<&Pool> {
        self.pools.get(pool_id)
    }

    pub fn get_lp_position(&self, pool_id: &PoolId, provider: &Address) -> Option<&LpPosition> {
        self.lp_positions.get(&(*pool_id, *provider))
    }

    /// Tüm pool'ları listele.
    ///
    /// Sıralama: Permanent Liquidity pool'lar önce, ardından hacme göre azalan.
    pub fn list_pools(&self) -> Vec<&Pool> {
        let mut pools: Vec<&Pool> = self.pools.values().collect();
        pools.sort_by(|a, b| {
            match (a.is_permanent(), b.is_permanent()) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => b.total_volume_a.cmp(&a.total_volume_a),
            }
        });
        pools
    }

    pub fn pool_count(&self) -> usize { self.pools.len() }
    pub fn treasury_balance(&self) -> Amount { self.treasury_balance }
    pub fn genesis_time(&self) -> Timestamp { self.genesis_time }
    pub fn payout_engine(&self) -> &CreatorPayoutEngine { &self.payout_engine }
    pub fn bot_counter(&self) -> &BotCounter { &self.bot_counter }

    // ─── Snapshot / Kalıcılık ─────────────────────────────────────────────────

    /// Tüm kayıt defteri durumunu bir snapshot'a dönüştür.
    ///
    /// nova-dex-server bu snapshot'ı RocksDB'ye bincode olarak yazar.
    #[cfg(feature = "serde")]
    pub fn to_snapshot(&self) -> crate::snapshot::RegistrySnapshot {
        use crate::snapshot::{
            LpPositionSnap, PoolSnap, RegistrySnapshot, SNAPSHOT_VERSION,
        };

        let pools = self
            .pools
            .values()
            .map(|p| PoolSnap {
                id:                   p.id.0,
                asset_a:              p.asset_a.0,
                asset_b:              p.asset_b.0,
                creator:              p.creator.0,
                reserve_a:            p.reserve_a,
                reserve_b:            p.reserve_b,
                total_lp:             p.total_lp,
                lock_creator:         p.lock_info.creator.0,
                lock_duration:        p.lock_info.duration.into(),
                lock_locked_at:       p.lock_info.locked_at,
                lock_unlock_at:       p.lock_info.unlock_at,
                created_at:           p.created_at,
                total_volume_a:       p.total_volume_a,
                total_fees_collected: p.total_fees_collected,
                swap_count:           p.swap_count,
            })
            .collect();

        let lp_positions = self
            .lp_positions
            .iter()
            .map(|((pool_id, _), pos)| LpPositionSnap {
                pool_id:     pool_id.0,
                provider:    pos.provider.0,
                lp_tokens:   pos.lp_tokens,
                deposited_a: pos.deposited_a,
                deposited_b: pos.deposited_b,
                created_at:  pos.created_at,
            })
            .collect();

        RegistrySnapshot {
            version:          SNAPSHOT_VERSION,
            genesis_time:     self.genesis_time,
            treasury_balance: self.treasury_balance,
            last_payout_at:   self.payout_engine.last_payout_at(),
            pools,
            lp_positions,
            bot_counters:    self.bot_counter.snapshot_entries(),
            pending_payouts: self.payout_engine.pending_entries(),
        }
    }

    /// Snapshot'tan kayıt defterini yeniden oluştur.
    ///
    /// Versiyon uyumsuzluğunda `Err(DexError::SnapshotVersionMismatch)` döner.
    #[cfg(feature = "serde")]
    pub fn from_snapshot(snap: crate::snapshot::RegistrySnapshot) -> Result<Self, crate::DexError> {
        use crate::snapshot::SNAPSHOT_VERSION;
        if snap.version != SNAPSHOT_VERSION {
            return Err(crate::DexError::SnapshotVersionMismatch {
                expected: SNAPSHOT_VERSION,
                found:    snap.version,
            });
        }
        use crate::liquidity::{LockInfo, LpPosition};
        use crate::types::{Address, AssetId, PoolId};

        let pools = snap
            .pools
            .into_iter()
            .map(|s| {
                let pool = Pool {
                    id:       PoolId(s.id),
                    asset_a:  AssetId(s.asset_a),
                    asset_b:  AssetId(s.asset_b),
                    creator:  Address(s.creator),
                    reserve_a: s.reserve_a,
                    reserve_b: s.reserve_b,
                    total_lp:  s.total_lp,
                    lock_info: LockInfo {
                        creator:   Address(s.lock_creator),
                        duration:  s.lock_duration.into(),
                        locked_at: s.lock_locked_at,
                        unlock_at: s.lock_unlock_at,
                    },
                    created_at:           s.created_at,
                    total_volume_a:       s.total_volume_a,
                    total_fees_collected: s.total_fees_collected,
                    swap_count:           s.swap_count,
                };
                (PoolId(s.id), pool)
            })
            .collect();

        let lp_positions = snap
            .lp_positions
            .into_iter()
            .map(|s| {
                let key = (PoolId(s.pool_id), Address(s.provider));
                let pos = LpPosition {
                    provider:    Address(s.provider),
                    lp_tokens:   s.lp_tokens,
                    deposited_a: s.deposited_a,
                    deposited_b: s.deposited_b,
                    created_at:  s.created_at,
                };
                (key, pos)
            })
            .collect();

        Ok(Self {
            pools,
            lp_positions,
            bot_counter:     crate::bot_counter::BotCounter::restore(snap.bot_counters),
            payout_engine:   crate::creator_payout::CreatorPayoutEngine::restore(
                snap.last_payout_at,
                snap.pending_payouts,
            ),
            treasury_balance: snap.treasury_balance,
            genesis_time:     snap.genesis_time,
        })
    }
}

// ─── TESTS ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{NOVA_DECIMALS_FACTOR as F, PAYOUT_CYCLE_SECS};

    fn nova(n: u128) -> Amount { n * F }

    fn alice()  -> Address { Address::from_seed(1) }
    fn bob()    -> Address { Address::from_seed(2) }
    fn nova_id() -> AssetId { AssetId::from_name("NOVA") }
    fn usdc_id() -> AssetId { AssetId::from_name("USDC") }

    const NOVA_PRICE_CENTS: u64 = 250; // $2.50

    fn default_registry() -> PoolRegistry { PoolRegistry::new(0) }

    fn create_default_pool(registry: &mut PoolRegistry) -> PoolId {
        registry.create_pool(
            alice(),
            nova_id(),
            usdc_id(),
            nova(1_000),
            nova(1_000),
            LockDuration::OneDay,
            nova(7),            // 7 NOVA = $17.50 at $2.50/NOVA
            NOVA_PRICE_CENTS,
            0,
        ).expect("pool oluşturma başarılı")
    }

    // ─── Pool Oluşturma Testleri ─────────────────────────────────────────

    /// POOL-FEE-001: PoolAlreadyExists hatası durumunda treasury artmamalı
    #[test]
    fn create_pool_duplicate_does_not_inflate_treasury() {
        let mut reg = default_registry();
        create_default_pool(&mut reg);
        let treasury_after_first = reg.treasury_balance();
        assert!(treasury_after_first > 0, "ilk pool oluşturmada treasury artmali");

        // Aynı pool tekrar oluşturulmaya çalışılıyor
        let err = reg.create_pool(
            alice(), nova_id(), usdc_id(),
            nova(1_000), nova(1_000),
            LockDuration::OneDay,
            nova(7), NOVA_PRICE_CENTS, 0,
        );
        assert!(matches!(err, Err(DexError::PoolAlreadyExists)));
        // Treasury değişmemiş olmalı
        assert_eq!(reg.treasury_balance(), treasury_after_first,
            "hata durumunda treasury artmamali");
    }

    #[test]
    fn create_pool_success() {
        let mut reg = default_registry();
        let pool_id = create_default_pool(&mut reg);
        assert_eq!(reg.pool_count(), 1);
        let pool = reg.get_pool(&pool_id).unwrap();
        assert_eq!(pool.creator, alice());
        assert_eq!(pool.reserve_a, nova(1_000));
        assert_eq!(pool.reserve_b, nova(1_000));
    }

    #[test]
    fn creation_fee_calculation() {
        // $2.50/NOVA → $17.50 için 7 NOVA gerekir
        let fee = required_creation_fee_nnova(250).unwrap();
        assert_eq!(fee, 7 * F);
    }

    #[test]
    fn creation_fee_at_different_prices() {
        // $100/NOVA → $17.50 için 0.175 NOVA = 175_000_000_000_000_000 nNOVA
        let fee = required_creation_fee_nnova(10_000).unwrap();
        assert_eq!(fee, (1_750 * F) / 10_000);
    }

    #[test]
    fn insufficient_creation_fee_rejected() {
        let mut reg = default_registry();
        let err = reg.create_pool(
            alice(), nova_id(), usdc_id(),
            nova(100), nova(100),
            LockDuration::OneDay,
            nova(1), // yetersiz fee
            NOVA_PRICE_CENTS,
            0,
        );
        assert!(matches!(err, Err(DexError::InsufficientCreationFee { .. })));
    }

    #[test]
    fn duplicate_pool_rejected() {
        let mut reg = default_registry();
        create_default_pool(&mut reg);
        let err = reg.create_pool(
            bob(), nova_id(), usdc_id(),
            nova(100), nova(100),
            LockDuration::OneDay,
            nova(7),
            NOVA_PRICE_CENTS,
            0,
        );
        assert_eq!(err, Err(DexError::PoolAlreadyExists));
    }

    #[test]
    fn same_token_pair_rejected() {
        let mut reg = default_registry();
        let err = reg.create_pool(
            alice(), nova_id(), nova_id(),
            nova(100), nova(100),
            LockDuration::OneDay,
            nova(7),
            NOVA_PRICE_CENTS,
            0,
        );
        assert_eq!(err, Err(DexError::InvalidTokenPair));
    }

    // ─── Swap Testleri ────────────────────────────────────────────────────

    #[test]
    fn swap_success_updates_reserves() {
        let mut reg = default_registry();
        let pool_id = create_default_pool(&mut reg);

        let comp = reg.swap(bob(), pool_id, true, nova(10), 0, 1_000).unwrap();
        assert!(comp.amount_out > 0);

        let pool = reg.get_pool(&pool_id).unwrap();
        // reserve_a arttı, reserve_b azaldı
        assert!(pool.reserve_a > nova(1_000));
        assert!(pool.reserve_b < nova(1_000));
        assert_eq!(pool.swap_count, 1);
    }

    #[test]
    fn swap_fee_distributed() {
        let mut reg = default_registry();
        let pool_id = create_default_pool(&mut reg);

        reg.swap(bob(), pool_id, true, nova(100), 0, 1_000).unwrap();

        // Treasury ve creator bekleyen bakiye var
        assert!(reg.treasury_balance() > 0);
        assert!(reg.payout_engine().pending_for(&alice()) > 0);
    }

    #[test]
    fn swap_slippage_enforced() {
        let mut reg = default_registry();
        let pool_id = create_default_pool(&mut reg);

        // %99 slippage koruması → çok düşük min_out
        let _comp = reg.swap(bob(), pool_id, true, nova(10), 0, 1_000).unwrap();
        // Yüksek min_out → red
        let err = reg.swap(bob(), pool_id, true, nova(10), nova(20), 1_000);
        assert!(matches!(err, Err(DexError::SlippageExceeded { .. })));
    }

    #[test]
    fn bot_discount_after_100_swaps() {
        let mut reg = default_registry();
        let pool_id = create_default_pool(&mut reg);

        let amount = nova(1);
        let now = 1_000u64;

        // 99 swap — normal fee
        for _ in 0..99 {
            reg.swap(bob(), pool_id, true, amount, 0, now).unwrap();
        }

        // 100. swap — bot fee (daha az fee kesilir)
        let comp_100 = reg.swap(bob(), pool_id, true, amount, 0, now).unwrap();
        assert_eq!(comp_100.fee_mode, crate::types::FeeMode::Bot);
    }

    // ─── Likidite Testleri ────────────────────────────────────────────────

    #[test]
    fn add_liquidity_increases_reserves() {
        let mut reg = default_registry();
        let pool_id = create_default_pool(&mut reg);

        let (lp, actual_a, actual_b) = reg.add_liquidity(bob(), pool_id, nova(100), nova(100), 100).unwrap();
        assert!(lp > 0);
        assert_eq!(actual_a, nova(100));
        assert_eq!(actual_b, nova(100));

        let pool = reg.get_pool(&pool_id).unwrap();
        assert_eq!(pool.reserve_a, nova(1_100));
        assert_eq!(pool.reserve_b, nova(1_100));
    }

    #[test]
    fn remove_liquidity_blocked_when_locked() {
        let mut reg = default_registry();
        let pool_id = create_default_pool(&mut reg); // OneDay lock

        // Kilit açılmadan önce çekme → hata
        let err = reg.remove_liquidity(alice(), pool_id, nova(100), 0);
        assert!(matches!(err, Err(DexError::LiquidityLocked { .. })));
    }

    #[test]
    fn remove_liquidity_after_lock_expires() {
        let mut reg = default_registry();
        let pool_id = create_default_pool(&mut reg); // OneDay lock, oluşturma zamanı = 0

        // 1 gün + 1 saniye sonra çekme
        let unlock_time = LockDuration::OneDay.to_seconds() + 1;
        let (a, b) = reg.remove_liquidity(alice(), pool_id, nova(100), unlock_time).unwrap();
        assert!(a > 0);
        assert!(b > 0);
    }

    #[test]
    fn rug_pull_impossible_unlimited() {
        let mut reg = PoolRegistry::new(0);
        // Unlimited lock → asla çekemez
        let pool_id = reg.create_pool(
            alice(), nova_id(), usdc_id(),
            nova(1_000), nova(1_000),
            LockDuration::Unlimited,
            nova(7),
            NOVA_PRICE_CENTS,
            0,
        ).unwrap();

        let err = reg.remove_liquidity(alice(), pool_id, nova(100), u64::MAX - 1);
        assert!(matches!(err, Err(DexError::LiquidityLocked { .. })));
    }

    // ─── Kilit Uzatma Testleri ────────────────────────────────────────────

    #[test]
    fn extend_lock_only_by_creator() {
        let mut reg = default_registry();
        let pool_id = create_default_pool(&mut reg);

        let err = reg.extend_lock(bob(), pool_id, LockDuration::OneWeek, 0);
        assert_eq!(err, Err(DexError::Unauthorized));
    }

    #[test]
    fn extend_lock_success() {
        let mut reg = default_registry();
        let pool_id = create_default_pool(&mut reg);

        reg.extend_lock(alice(), pool_id, LockDuration::OneWeek, 0).unwrap();
        let pool = reg.get_pool(&pool_id).unwrap();
        assert_eq!(pool.lock_info.duration, LockDuration::OneWeek);
    }

    // ─── Creator Ödeme Testleri ───────────────────────────────────────────

    #[test]
    fn creator_payout_after_24h() {
        let mut reg = default_registry();
        let pool_id = create_default_pool(&mut reg);

        // Swap yap → fee birikir
        reg.swap(bob(), pool_id, true, nova(100), 0, 1_000).unwrap();

        let pending_before = reg.payout_engine().pending_for(&alice());
        assert!(pending_before > 0);

        // 24 saat geçince ödeme işlenir
        let records = reg.process_creator_payouts(PAYOUT_CYCLE_SECS).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].creator, alice());
        assert_eq!(records[0].gross_amount, pending_before);
        assert_eq!(reg.payout_engine().pending_for(&alice()), 0);
    }

    // ─── Pool Listeleme Testleri ──────────────────────────────────────────

    #[test]
    fn permanent_pools_listed_first() {
        let mut reg = PoolRegistry::new(0);
        let btc_id = AssetId::from_name("BTC");

        // Normal pool (OneDay lock)
        reg.create_pool(alice(), nova_id(), usdc_id(), nova(100), nova(100),
            LockDuration::OneDay, nova(7), NOVA_PRICE_CENTS, 0).unwrap();

        // Permanent pool
        reg.create_pool(alice(), nova_id(), btc_id, nova(100), nova(100),
            LockDuration::Unlimited, nova(7), NOVA_PRICE_CENTS, 0).unwrap();

        let pools = reg.list_pools();
        assert_eq!(pools.len(), 2);
        assert!(pools[0].is_permanent(), "Permanent pool önce gelmeli");
    }
}
