//! AUTH-001 — k256 ECDSA İmza Doğrulama Modülü.
//!
//! ## Şema
//!
//! 1. İstemci, işlem parametrelerini domain-ayrılmış bir mesaj olarak SHA-256 ile hashler.
//! 2. İstemci, secp256k1 özel anahtarıyla bu hash'i ECDSA ile imzalar.
//! 3. İstemci, `public_key` (hex compressed, 33 byte) ve `signature` (hex compact, 64 byte)
//!    ile birlikte isteği gönderir.
//! 4. Sunucu, `SHA256(public_key) == address` ve imza geçerliliğini doğrular.
//! 5. Sunucu, `timestamp_secs` içinde ± [`TIMESTAMP_TOLERANCE_SECS`] aralığını kontrol eder
//!    (replay saldırısı koruması).
//!
//! ## Mesaj Formatı (her işlem için)
//!
//! ```text
//! SHA256(domain_separator || field_1_bytes || field_2_bytes || ...)
//! ```
//!
//! Domain separator işlem türünü (swap, likidite vs.) birbirinden ayırır;
//! farklı işlemler arası cross-replay saldırısı imkânsız hale gelir.

use k256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
use nova_dex::types::{Address, AssetId, LockDuration, PoolId};
use sha2::Digest;

use crate::error::ApiError;

/// İmza geçerlilik penceresi (saniye). İstek ± bu süre dışındaysa reddedilir.
pub const TIMESTAMP_TOLERANCE_SECS: u64 = 60;

// ─── YARDIMCI ────────────────────────────────────────────────────────────────

/// Compressed secp256k1 public key'den Nova adresi türet:
/// `SHA256("nova_addr:" ‖ pubkey[33])`.
///
/// Domain prefix, bu hash'in başka SHA-256 kullanımlarıyla çakışmasını önler.
pub fn address_from_pubkey(pubkey_bytes: &[u8]) -> [u8; 32] {
    let mut h = sha2::Sha256::new();
    h.update(b"nova_addr:");
    h.update(pubkey_bytes);
    h.finalize().into()
}

/// Temel ECDSA doğrulama.
///
/// - Pubkey: hex-encoded compressed secp256k1 (33 byte → 66 hex karakter).
/// - Signature: hex-encoded compact ECDSA (64 byte → 128 hex karakter).
/// - Address tutarlılığı: `SHA256(pubkey) == address.0`.
/// - Timestamp replay koruması: `|now - timestamp_secs| <= TIMESTAMP_TOLERANCE_SECS`.
fn verify_inner(
    address: &Address,
    pubkey_hex: &str,
    sig_hex: &str,
    timestamp_secs: u64,
    now_secs: u64,
    msg: &[u8],
) -> Result<(), ApiError> {
    // 1. Timestamp penceresi kontrolü (replay koruması)
    let diff = now_secs.abs_diff(timestamp_secs);
    if diff > TIMESTAMP_TOLERANCE_SECS {
        return Err(ApiError::invalid_input(format!(
            "İstek zaman damgası kabul penceresi dışında ({diff}s > {TIMESTAMP_TOLERANCE_SECS}s)"
        )));
    }

    // 2. Public key decode
    let pubkey_bytes = hex::decode(pubkey_hex)
        .map_err(|_| ApiError::invalid_input("public_key: geçersiz hex"))?;
    if pubkey_bytes.len() != 33 {
        return Err(ApiError::invalid_input("public_key: 33 byte compressed key gerekli"));
    }

    let verifying_key = VerifyingKey::from_sec1_bytes(&pubkey_bytes)
        .map_err(|_| ApiError::invalid_input("public_key: geçersiz secp256k1 key"))?;

    // 3. Adres tutarlılığı: SHA256(pubkey) == address
    let derived = address_from_pubkey(&pubkey_bytes);
    if derived != address.0 {
        return Err(ApiError::Unauthorized);
    }

    // 4. Signature decode (64 byte compact: r ‖ s)
    let sig_bytes = hex::decode(sig_hex)
        .map_err(|_| ApiError::invalid_input("signature: geçersiz hex"))?;
    let sig = Signature::try_from(sig_bytes.as_slice())
        .map_err(|_| ApiError::invalid_input("signature: 64 byte compact format gerekli"))?;

    // 5. Doğrula (k256 mesajı SHA256 ile hashler)
    verifying_key
        .verify(msg, &sig)
        .map_err(|_| ApiError::Unauthorized)
}

// ─── MESAJ OLUŞTURUCULARI ────────────────────────────────────────────────────

/// MEV reveal (swap) işlemi için canonical mesaj.
///
/// Nonce dahil edilir → commitment hash ile bağlantı sağlanır.
#[allow(clippy::too_many_arguments)]
fn swap_msg(
    sender: &Address,
    pool_id: &PoolId,
    asset_in: &AssetId,
    asset_out: &AssetId,
    amount_in: u128,
    min_amount_out: u128,
    nonce: &[u8; 32],
    timestamp_secs: u64,
) -> Vec<u8> {
    let mut v = Vec::with_capacity(19 + 32 + 32 + 32 + 32 + 16 + 16 + 32 + 8);
    v.extend_from_slice(b"nova:mev_reveal:v1:");
    v.extend_from_slice(&sender.0);
    v.extend_from_slice(&pool_id.0);
    v.extend_from_slice(&asset_in.0);
    v.extend_from_slice(&asset_out.0);
    v.extend_from_slice(&amount_in.to_le_bytes());
    v.extend_from_slice(&min_amount_out.to_le_bytes());
    v.extend_from_slice(nonce);
    v.extend_from_slice(&timestamp_secs.to_le_bytes());
    v
}

/// `add_liquidity` işlemi için canonical mesaj.
fn add_liquidity_msg(
    pool_id: &PoolId,
    provider: &Address,
    amount_a: u128,
    amount_b: u128,
    timestamp_secs: u64,
) -> Vec<u8> {
    let mut v = Vec::with_capacity(16 + 32 + 32 + 16 + 16 + 8);
    v.extend_from_slice(b"nova:add_liq:v1:");
    v.extend_from_slice(&pool_id.0);
    v.extend_from_slice(&provider.0);
    v.extend_from_slice(&amount_a.to_le_bytes());
    v.extend_from_slice(&amount_b.to_le_bytes());
    v.extend_from_slice(&timestamp_secs.to_le_bytes());
    v
}

/// `remove_liquidity` işlemi için canonical mesaj.
fn remove_liquidity_msg(
    pool_id: &PoolId,
    provider: &Address,
    lp_to_burn: u128,
    timestamp_secs: u64,
) -> Vec<u8> {
    let mut v = Vec::with_capacity(16 + 32 + 32 + 16 + 8);
    v.extend_from_slice(b"nova:rem_liq:v1:");
    v.extend_from_slice(&pool_id.0);
    v.extend_from_slice(&provider.0);
    v.extend_from_slice(&lp_to_burn.to_le_bytes());
    v.extend_from_slice(&timestamp_secs.to_le_bytes());
    v
}

/// `extend_lock` işlemi için canonical mesaj.
fn extend_lock_msg(
    pool_id: &PoolId,
    caller: &Address,
    new_duration: LockDuration,
    timestamp_secs: u64,
) -> Vec<u8> {
    let dur_byte: u8 = match new_duration {
        LockDuration::OneDay       => 0,
        LockDuration::ThreeDays    => 1,
        LockDuration::OneWeek      => 2,
        LockDuration::OneMonth     => 3,
        LockDuration::TwelveMonths => 4,
        LockDuration::Unlimited    => 5,
    };
    let mut v = Vec::with_capacity(16 + 32 + 32 + 1 + 8);
    v.extend_from_slice(b"nova:ext_lock:v1:");
    v.extend_from_slice(&pool_id.0);
    v.extend_from_slice(&caller.0);
    v.push(dur_byte);
    v.extend_from_slice(&timestamp_secs.to_le_bytes());
    v
}

/// `create_pool` işlemi için canonical mesaj.
fn create_pool_msg(
    creator: &Address,
    asset_a: &AssetId,
    asset_b: &AssetId,
    initial_a: u128,
    initial_b: u128,
    creation_fee: u128,
    timestamp_secs: u64,
) -> Vec<u8> {
    let mut v = Vec::with_capacity(17 + 32 + 32 + 32 + 16 + 16 + 16 + 8);
    v.extend_from_slice(b"nova:create_pool:v1:");
    v.extend_from_slice(&creator.0);
    v.extend_from_slice(&asset_a.0);
    v.extend_from_slice(&asset_b.0);
    v.extend_from_slice(&initial_a.to_le_bytes());
    v.extend_from_slice(&initial_b.to_le_bytes());
    v.extend_from_slice(&creation_fee.to_le_bytes());
    v.extend_from_slice(&timestamp_secs.to_le_bytes());
    v
}

// ─── PUBLIC API ──────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn verify_swap(
    sender: &Address,
    pool_id: &PoolId,
    asset_in: &AssetId,
    asset_out: &AssetId,
    pubkey_hex: &str,
    sig_hex: &str,
    amount_in: u128,
    min_amount_out: u128,
    nonce: &[u8; 32],
    timestamp_secs: u64,
    now_secs: u64,
) -> Result<(), ApiError> {
    let msg = swap_msg(sender, pool_id, asset_in, asset_out, amount_in, min_amount_out, nonce, timestamp_secs);
    verify_inner(sender, pubkey_hex, sig_hex, timestamp_secs, now_secs, &msg)
}

#[allow(clippy::too_many_arguments)]
pub fn verify_add_liquidity(
    pool_id: &PoolId,
    provider: &Address,
    pubkey_hex: &str,
    sig_hex: &str,
    amount_a: u128,
    amount_b: u128,
    timestamp_secs: u64,
    now_secs: u64,
) -> Result<(), ApiError> {
    let msg = add_liquidity_msg(pool_id, provider, amount_a, amount_b, timestamp_secs);
    verify_inner(provider, pubkey_hex, sig_hex, timestamp_secs, now_secs, &msg)
}

pub fn verify_remove_liquidity(
    pool_id: &PoolId,
    provider: &Address,
    pubkey_hex: &str,
    sig_hex: &str,
    lp_to_burn: u128,
    timestamp_secs: u64,
    now_secs: u64,
) -> Result<(), ApiError> {
    let msg = remove_liquidity_msg(pool_id, provider, lp_to_burn, timestamp_secs);
    verify_inner(provider, pubkey_hex, sig_hex, timestamp_secs, now_secs, &msg)
}

pub fn verify_extend_lock(
    pool_id: &PoolId,
    caller: &Address,
    pubkey_hex: &str,
    sig_hex: &str,
    new_duration: LockDuration,
    timestamp_secs: u64,
    now_secs: u64,
) -> Result<(), ApiError> {
    let msg = extend_lock_msg(pool_id, caller, new_duration, timestamp_secs);
    verify_inner(caller, pubkey_hex, sig_hex, timestamp_secs, now_secs, &msg)
}

#[allow(clippy::too_many_arguments)]
pub fn verify_create_pool(
    creator: &Address,
    asset_a: &AssetId,
    asset_b: &AssetId,
    pubkey_hex: &str,
    sig_hex: &str,
    initial_a: u128,
    initial_b: u128,
    creation_fee: u128,
    timestamp_secs: u64,
    now_secs: u64,
) -> Result<(), ApiError> {
    let msg = create_pool_msg(creator, asset_a, asset_b, initial_a, initial_b, creation_fee, timestamp_secs);
    verify_inner(creator, pubkey_hex, sig_hex, timestamp_secs, now_secs, &msg)
}

// ─── TESTLER ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::{signature::Signer, SigningKey};
    use nova_dex::types::PoolId;

    /// Deterministik test anahtar çifti (sabit seed = 1).
    fn test_signing_key() -> SigningKey {
        SigningKey::from_bytes(&[1u8; 32].into()).expect("geçerli signing key")
    }

    fn pubkey_hex(sk: &SigningKey) -> String {
        let pk_bytes: Vec<u8> = sk.verifying_key().to_sec1_bytes().into();
        hex::encode(pk_bytes)
    }

    fn sign_hex(sk: &SigningKey, msg: &[u8]) -> String {
        let sig: k256::ecdsa::Signature = sk.sign(msg);
        hex::encode(sig.to_bytes())
    }

    fn test_address(sk: &SigningKey) -> Address {
        let pk_bytes: Vec<u8> = sk.verifying_key().to_sec1_bytes().into();
        Address(address_from_pubkey(&pk_bytes))
    }

    #[test]
    fn address_from_pubkey_uses_domain_prefix() {
        let sk = test_signing_key();
        let pk_bytes: Vec<u8> = sk.verifying_key().to_sec1_bytes().into();

        let with_prefix = address_from_pubkey(&pk_bytes);
        // SHA256(pubkey) olmadan — farklı olmalı
        let raw: [u8; 32] = sha2::Sha256::digest(&pk_bytes).into();
        assert_ne!(with_prefix, raw, "prefix olmalı");
    }

    #[test]
    fn address_from_pubkey_deterministic() {
        let sk = test_signing_key();
        let pk_bytes: Vec<u8> = sk.verifying_key().to_sec1_bytes().into();
        assert_eq!(address_from_pubkey(&pk_bytes), address_from_pubkey(&pk_bytes));
    }

    #[test]
    fn verify_add_liquidity_ok() {
        let sk = test_signing_key();
        let pool_id  = PoolId([2u8; 32]);
        let provider = test_address(&sk);
        let now      = 1_000_000u64;
        let msg      = super::add_liquidity_msg(&pool_id, &provider, 100, 200, now);

        let pk_hex  = pubkey_hex(&sk);
        let sig_hex = sign_hex(&sk, &msg);

        let result = verify_add_liquidity(&pool_id, &provider, &pk_hex, &sig_hex, 100, 200, now, now);
        assert!(result.is_ok(), "geçerli imza reddedilmemeli: {result:?}");
    }

    #[test]
    fn verify_fails_wrong_signature() {
        let sk       = test_signing_key();
        let pool_id  = PoolId([2u8; 32]);
        let provider = test_address(&sk);
        let now      = 1_000_000u64;
        let pk_hex   = pubkey_hex(&sk);
        let bad_sig  = hex::encode([0u8; 64]);

        let result = verify_add_liquidity(&pool_id, &provider, &pk_hex, &bad_sig, 100, 200, now, now);
        assert!(result.is_err(), "sahte imza kabul edilmemeli");
    }

    #[test]
    fn verify_fails_wrong_address() {
        let sk       = test_signing_key();
        let pool_id  = PoolId([2u8; 32]);
        // Farklı adres — pubkey ile eşleşmiyor
        let wrong_address = Address([99u8; 32]);
        let now      = 1_000_000u64;
        let msg      = super::add_liquidity_msg(&pool_id, &wrong_address, 100, 200, now);
        let pk_hex   = pubkey_hex(&sk);
        let sig_hex  = sign_hex(&sk, &msg);

        let result = verify_add_liquidity(&pool_id, &wrong_address, &pk_hex, &sig_hex, 100, 200, now, now);
        assert!(result.is_err(), "adres uyuşmazlığı reddedilmeli");
    }

    #[test]
    fn verify_fails_expired_timestamp() {
        let sk       = test_signing_key();
        let pool_id  = PoolId([2u8; 32]);
        let provider = test_address(&sk);
        let old_ts   = 1_000_000u64;
        let now      = old_ts + TIMESTAMP_TOLERANCE_SECS + 1;
        let msg      = super::add_liquidity_msg(&pool_id, &provider, 100, 200, old_ts);
        let pk_hex   = pubkey_hex(&sk);
        let sig_hex  = sign_hex(&sk, &msg);

        let result = verify_add_liquidity(&pool_id, &provider, &pk_hex, &sig_hex, 100, 200, old_ts, now);
        assert!(result.is_err(), "süresi dolmuş timestamp reddedilmeli");
    }

    #[test]
    fn verify_fails_future_timestamp() {
        let sk       = test_signing_key();
        let pool_id  = PoolId([2u8; 32]);
        let provider = test_address(&sk);
        let now      = 1_000_000u64;
        let future   = now + TIMESTAMP_TOLERANCE_SECS + 1;
        let msg      = super::add_liquidity_msg(&pool_id, &provider, 100, 200, future);
        let pk_hex   = pubkey_hex(&sk);
        let sig_hex  = sign_hex(&sk, &msg);

        let result = verify_add_liquidity(&pool_id, &provider, &pk_hex, &sig_hex, 100, 200, future, now);
        assert!(result.is_err(), "gelecek timestamp reddedilmeli");
    }
}
