//! Nova DEX — AMM Çekirdeği: Constant Product Market Maker (x · y = k).
//!
//! ## Formüller
//!
//! **Swap çıktısı (fee sonrası net giriş kullanılarak):**
//! ```text
//! fee_amount        = amount_in × fee_bps / 10_000
//! amount_in_net     = amount_in − fee_amount
//! amount_out        = (reserve_out × amount_in_net)
//!                     ─────────────────────────────────
//!                     (reserve_in  + amount_in_net)
//! ```
//!
//! **Fiyat etkisi (bps):**
//! ```text
//! spot_price   = reserve_out / reserve_in
//! exec_price   = amount_out  / amount_in_net
//! impact_bps   = (spot − exec) / spot × 10_000
//! ```
//!
//! ## Taşma Koruması
//!
//! `reserve × amount` çarpımı u128 sınırını aşabilir; bu nedenle kritik
//! çarpımlar [`primitive_types::U256`] üzerinde yapılır, sonuç u128'e
//! indirgenir (reserve_out'tan küçük olacağı garanti).

use primitive_types::U256;

use crate::error::DexError;
use crate::types::{Amount, FeeMode};

// ─── SWAP SONUCU ─────────────────────────────────────────────────────────────

/// Tek bir swap işleminin tam hesaplama sonucu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapComputation {
    /// Kullanıcının gönderdiği brüt giriş miktarı.
    pub amount_in: Amount,
    /// Brüt girişten kesilen fee.
    pub fee_amount: Amount,
    /// Fee sonrası net giriş (AMM formülünde kullanılan).
    pub amount_in_net: Amount,
    /// Kullanıcıya verilen çıkış miktarı.
    pub amount_out: Amount,
    /// Fiyat etkisi, bps cinsinden.
    pub price_impact_bps: u128,
    /// Uygulanan fee modu.
    pub fee_mode: FeeMode,
}

impl SwapComputation {
    /// Creator'a giden fee payı (%20).
    #[inline]
    pub fn creator_fee(&self) -> Amount {
        self.fee_amount * 20 / 100
    }

    /// Kasaya (treasury) giden fee payı (%80).
    /// Creator ile toplam tam eşitlik: treasury = fee - creator.
    #[inline]
    pub fn treasury_fee(&self) -> Amount {
        self.fee_amount - self.creator_fee()
    }
}

// ─── TEMEL AMM FONKSİYONLARI ─────────────────────────────────────────────────

/// x · y = k swap hesabı.
///
/// # Parametreler
/// * `reserve_in`  — Giriş token rezervi
/// * `reserve_out` — Çıkış token rezervi
/// * `amount_in`   — Brüt giriş (fee dahil)
/// * `fee_mode`    — Normal (%0.30) veya Bot (%0.15)
///
/// # Hatalar
/// * [`DexError::ZeroAmount`] — `amount_in == 0`
/// * [`DexError::ZeroReserve`] — herhangi bir rezerv sıfırsa
/// * [`DexError::InsufficientLiquidity`] — `amount_out` hesaplanamıyorsa
pub fn compute_swap(
    reserve_in: Amount,
    reserve_out: Amount,
    amount_in: Amount,
    fee_mode: FeeMode,
) -> Result<SwapComputation, DexError> {
    if amount_in == 0 {
        return Err(DexError::ZeroAmount);
    }
    if reserve_in == 0 || reserve_out == 0 {
        return Err(DexError::ZeroReserve);
    }

    let fee_bps = fee_mode.fee_bps();

    // fee_amount = amount_in × fee_bps / 10_000
    let fee_amount = amount_in
        .checked_mul(fee_bps)
        .ok_or(DexError::Overflow)?
        .checked_div(10_000)
        .ok_or(DexError::Overflow)?;

    let amount_in_net = amount_in
        .checked_sub(fee_amount)
        .ok_or(DexError::Overflow)?;

    // U256 kullanarak taşma riski olmadan hesapla
    let amount_out = {
        let r_out = U256::from(reserve_out);
        let a_in  = U256::from(amount_in_net);
        let r_in  = U256::from(reserve_in);

        let numerator   = r_out * a_in;
        let denominator = r_in + a_in;

        if denominator.is_zero() {
            return Err(DexError::Overflow);
        }

        let result = numerator / denominator;
        // Sonuç her zaman reserve_out'tan küçük — u128'e güvenli indirge
        result.low_u128()
    };

    if amount_out == 0 {
        return Err(DexError::InsufficientLiquidity { reserve_in, reserve_out });
    }

    let price_impact_bps = compute_price_impact_bps(
        reserve_in,
        reserve_out,
        amount_in_net,
        amount_out,
    )?;

    Ok(SwapComputation {
        amount_in,
        fee_amount,
        amount_in_net,
        amount_out,
        price_impact_bps,
        fee_mode,
    })
}

/// Fiyat etkisini bps cinsinden hesapla.
///
/// ```text
/// spot_price_scaled = reserve_out × SCALE / reserve_in
/// exec_price_scaled = amount_out  × SCALE / amount_in_net
/// impact_bps        = (spot − exec) / spot × 10_000
/// ```
fn compute_price_impact_bps(
    reserve_in: Amount,
    reserve_out: Amount,
    amount_in_net: Amount,
    amount_out: Amount,
) -> Result<u128, DexError> {
    const SCALE: u128 = 1_000_000_000_000u128; // 10^12

    let spot = U256::from(reserve_out)
        .checked_mul(U256::from(SCALE))
        .ok_or(DexError::Overflow)?
        .checked_div(U256::from(reserve_in))
        .ok_or(DexError::Overflow)?;

    if spot.is_zero() {
        return Ok(0);
    }

    let exec = U256::from(amount_out)
        .checked_mul(U256::from(SCALE))
        .ok_or(DexError::Overflow)?
        .checked_div(U256::from(amount_in_net))
        .ok_or(DexError::Overflow)?;

    if exec >= spot {
        return Ok(0);
    }

    let impact = (spot - exec)
        .checked_mul(U256::from(10_000u64))
        .ok_or(DexError::Overflow)?
        .checked_div(spot)
        .ok_or(DexError::Overflow)?;

    Ok(impact.low_u128())
}

// ─── SLIPPAGE ─────────────────────────────────────────────────────────────────

/// Slippage toleransına göre minimum kabul edilebilir çıktıyı hesapla.
///
/// `min_out = expected_out × (10_000 − slippage_bps) / 10_000`
pub fn min_amount_out_for_slippage(
    expected_out: Amount,
    slippage_bps: u128,
) -> Result<Amount, DexError> {
    if expected_out == 0 {
        return Err(DexError::ZeroAmount);
    }
    let slippage = slippage_bps.min(9_999);
    expected_out
        .checked_mul(10_000 - slippage)
        .ok_or(DexError::Overflow)?
        .checked_div(10_000)
        .ok_or(DexError::Overflow)
}

// ─── SPOT FİYAT ──────────────────────────────────────────────────────────────

/// Spot fiyatı döner: `(reserve_out × PRICE_PRECISION) / reserve_in`.
///
/// Kesirden kaçınmak için `PRICE_PRECISION = 10^18` ile ölçeklendirilir.
pub const PRICE_PRECISION: u128 = NOVA_DECIMALS_FACTOR;
use crate::types::NOVA_DECIMALS_FACTOR;

pub fn spot_price(reserve_in: Amount, reserve_out: Amount) -> Result<u128, DexError> {
    if reserve_in == 0 {
        return Err(DexError::ZeroReserve);
    }
    let result = U256::from(reserve_out)
        .checked_mul(U256::from(PRICE_PRECISION))
        .ok_or(DexError::Overflow)?
        .checked_div(U256::from(reserve_in))
        .ok_or(DexError::Overflow)?;
    Ok(result.low_u128())
}

// ─── LP TOKEN HESABI ─────────────────────────────────────────────────────────

/// LP token mint miktarını hesapla.
///
/// - İlk yatırım: `sqrt(amount_a × amount_b)` (geometrik ortalama)
/// - Sonraki yatırımlar: `min(a × total_lp / reserve_a, b × total_lp / reserve_b)`
pub fn compute_lp_tokens_to_mint(
    amount_a: Amount,
    amount_b: Amount,
    reserve_a: Amount,
    reserve_b: Amount,
    total_lp: Amount,
) -> Result<Amount, DexError> {
    if amount_a == 0 || amount_b == 0 {
        return Err(DexError::ZeroAmount);
    }

    if total_lp == 0 {
        // İlk yatırım: LP = isqrt(a × b)
        let product = U256::from(amount_a)
            .checked_mul(U256::from(amount_b))
            .ok_or(DexError::Overflow)?;
        let lp = u256_isqrt(product);
        Ok(lp.low_u128())
    } else {
        // Orantılı yatırım
        let lp_a = U256::from(amount_a)
            .checked_mul(U256::from(total_lp))
            .ok_or(DexError::Overflow)?
            .checked_div(U256::from(reserve_a))
            .ok_or(DexError::Overflow)?;

        let lp_b = U256::from(amount_b)
            .checked_mul(U256::from(total_lp))
            .ok_or(DexError::Overflow)?
            .checked_div(U256::from(reserve_b))
            .ok_or(DexError::Overflow)?;

        Ok(lp_a.min(lp_b).low_u128())
    }
}

/// Verilen miktarları mevcut pool oranına normalize et.
///
/// İlk yatırımda (total_lp == 0) normalize gerekmez, (amount_a, amount_b) olduğu gibi döner.
/// Sonraki yatırımlarda bağlayıcı taraf belirlenir; fazla token iade edilmek üzere azaltılır.
///
/// Döner: `(actual_a, actual_b)` — pool'a gerçekten eklenecek miktarlar.
pub fn normalize_liquidity(
    amount_a: Amount,
    amount_b: Amount,
    reserve_a: Amount,
    reserve_b: Amount,
    total_lp: Amount,
) -> Result<(Amount, Amount), DexError> {
    if total_lp == 0 || reserve_a == 0 || reserve_b == 0 {
        return Ok((amount_a, amount_b));
    }

    // A'ya göre ideal B: ideal_b = reserve_b * amount_a / reserve_a
    let ideal_b = (U256::from(reserve_b)
        .checked_mul(U256::from(amount_a))
        .ok_or(DexError::Overflow)?
        / U256::from(reserve_a))
    .low_u128();

    if ideal_b <= amount_b {
        // A bağlayıcı kısıt: tüm A kullan, ideal_b kadar B kullan
        Ok((amount_a, ideal_b))
    } else {
        // B bağlayıcı kısıt: tüm B kullan, ideal_a kadar A kullan
        let ideal_a = (U256::from(reserve_a)
            .checked_mul(U256::from(amount_b))
            .ok_or(DexError::Overflow)?
            / U256::from(reserve_b))
        .low_u128();
        Ok((ideal_a, amount_b))
    }
}

/// LP token yakıldığında geri alınacak miktarları hesapla.
pub fn compute_withdrawal_amounts(
    lp_to_burn: Amount,
    total_lp: Amount,
    reserve_a: Amount,
    reserve_b: Amount,
) -> Result<(Amount, Amount), DexError> {
    if lp_to_burn == 0 {
        return Err(DexError::ZeroAmount);
    }
    if lp_to_burn > total_lp {
        return Err(DexError::InsufficientBalance {
            available: total_lp,
            requested: lp_to_burn,
        });
    }

    let amount_a = (U256::from(lp_to_burn) * U256::from(reserve_a) / U256::from(total_lp))
        .low_u128();
    let amount_b = (U256::from(lp_to_burn) * U256::from(reserve_b) / U256::from(total_lp))
        .low_u128();

    Ok((amount_a, amount_b))
}

// ─── YARDIMCI: U256 TAM KAREKÖKU ─────────────────────────────────────────────

/// Newton yöntemiyle U256 tam kareköku.
fn u256_isqrt(n: U256) -> U256 {
    if n.is_zero() {
        return U256::zero();
    }
    let two = U256::from(2u64);
    let mut x = n;
    let mut y = (x + U256::from(1u64)) / two;
    while y < x {
        x = y;
        y = (x + n / x) / two;
    }
    x
}

// ─── TESTS ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NOVA_DECIMALS_FACTOR as F;

    fn nova(n: u128) -> Amount {
        n * F
    }

    #[test]
    fn swap_basic_constant_product() {
        // Pool: 100 NOVA / 100 USDC, normal fee %0.30
        let reserve_in  = nova(100);
        let reserve_out = nova(100);
        let amount_in   = nova(10);

        let result = compute_swap(reserve_in, reserve_out, amount_in, FeeMode::Normal)
            .expect("swap başarılı olmalı");

        // amount_out beklenen değer
        // fee = 10 * 30 / 10000 = 0.03 NOVA
        // net_in = 9.97 NOVA
        // out = (100 * 9.97) / (100 + 9.97) ≈ 9.066 NOVA
        assert!(result.amount_out > 0, "çıkış pozitif olmalı");
        assert!(result.amount_out < nova(10), "çıkış giriş'ten az olmalı (fee + slippage)");
        assert_eq!(result.fee_mode, FeeMode::Normal);
        assert!(result.price_impact_bps > 0, "fiyat etkisi olmalı");

        // creator+treasury = toplam fee
        assert_eq!(
            result.creator_fee() + result.treasury_fee(),
            result.fee_amount
        );
    }

    #[test]
    fn swap_bot_mode_lower_fee() {
        let reserve_in  = nova(1_000);
        let reserve_out = nova(1_000);
        let amount_in   = nova(10);

        let normal = compute_swap(reserve_in, reserve_out, amount_in, FeeMode::Normal).unwrap();
        let bot    = compute_swap(reserve_in, reserve_out, amount_in, FeeMode::Bot).unwrap();

        // Bot fee daha düşük → daha az fee, daha fazla net giriş → daha fazla çıkış
        assert!(bot.fee_amount < normal.fee_amount, "bot fee daha düşük olmalı");
        assert!(bot.amount_out > normal.amount_out, "bot çıkışı daha fazla olmalı");
    }

    #[test]
    fn swap_fee_split_20_80() {
        let result = compute_swap(nova(1_000), nova(1_000), nova(100), FeeMode::Normal).unwrap();

        let creator  = result.creator_fee();
        let treasury = result.treasury_fee();
        let total    = result.fee_amount;

        // %20 creator, %80 treasury
        assert_eq!(creator, total * 20 / 100);
        assert_eq!(creator + treasury, total);
    }

    #[test]
    fn swap_zero_amount_err() {
        assert_eq!(
            compute_swap(nova(100), nova(100), 0, FeeMode::Normal),
            Err(DexError::ZeroAmount)
        );
    }

    #[test]
    fn swap_zero_reserve_err() {
        assert_eq!(
            compute_swap(0, nova(100), nova(10), FeeMode::Normal),
            Err(DexError::ZeroReserve)
        );
    }

    #[test]
    fn swap_large_amounts_no_overflow() {
        // 10 milyon NOVA her rezervde — u128 doğrudan taşardı, U256 ile güvenli
        let reserve = 10_000_000 * F;
        let amount  = 100_000 * F;
        let result  = compute_swap(reserve, reserve, amount, FeeMode::Normal);
        assert!(result.is_ok(), "büyük miktarlarda taşma olmamalı: {:?}", result);
    }

    #[test]
    fn lp_tokens_first_deposit() {
        // İlk yatırım: sqrt(100 × 100) = 100 NOVA LP
        let lp = compute_lp_tokens_to_mint(nova(100), nova(100), 0, 0, 0).unwrap();
        // sqrt(100 * 10^18 * 100 * 10^18) = sqrt(10000 * 10^36) = 100 * 10^18
        assert_eq!(lp, nova(100));
    }

    #[test]
    fn lp_tokens_subsequent_deposit() {
        // Mevcut pool: 100 NOVA / 100 USDC, 100 LP
        // 10 NOVA + 10 USDC ekle → 10 LP beklenir
        let lp = compute_lp_tokens_to_mint(
            nova(10), nova(10),
            nova(100), nova(100),
            nova(100),
        )
        .unwrap();
        assert_eq!(lp, nova(10));
    }

    #[test]
    fn withdrawal_amounts_correct() {
        // 50 LP yak → %50 oranında geri alım
        let (a, b) = compute_withdrawal_amounts(
            nova(50), nova(100), nova(200), nova(400)
        )
        .unwrap();
        assert_eq!(a, nova(100)); // 50/100 * 200 = 100
        assert_eq!(b, nova(200)); // 50/100 * 400 = 200
    }

    #[test]
    fn spot_price_symmetric() {
        // 1000 NOVA / 500 USDC → 1 NOVA = 0.5 USDC
        let price = spot_price(nova(1_000), nova(500)).unwrap();
        // price = (500 × PRECISION) / 1000 = 0.5 × PRECISION
        assert_eq!(price, F / 2);
    }

    #[test]
    fn min_amount_out_five_percent_slippage() {
        let expected = nova(100);
        let min = min_amount_out_for_slippage(expected, 500).unwrap(); // %5 slippage
        assert_eq!(min, nova(95)); // %95 of 100
    }

    #[test]
    fn price_impact_large_swap() {
        // Büyük swap → yüksek fiyat etkisi
        let r = nova(100);
        let small = compute_swap(r, r, nova(1), FeeMode::Normal).unwrap();
        let large = compute_swap(r, r, nova(50), FeeMode::Normal).unwrap();
        assert!(
            large.price_impact_bps > small.price_impact_bps,
            "büyük swap daha yüksek fiyat etkisi üretmeli"
        );
    }
}
