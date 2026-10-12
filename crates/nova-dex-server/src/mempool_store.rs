//! Nova DEX — Redis Tabanlı MEV Korumalı Mempool.
//!
//! ## Tasarım
//!
//! Commit-reveal mekanizması Redis üzerinde çalışır; böylece tüm sunucu
//! instance'ları aynı mempool'u paylaşır (horizontal scaling).
//!
//! ### Anahtar yapısı
//! ```text
//! nova:mev:commit:{hex_hash}  →  JSON {sender, slot_id, committed_at_ms, revealed}
//! nova:mev:intent:{hex_hash}  →  JSON {tam SwapIntent alanları}
//! nova:mev:slot:{slot_id}     →  Redis Sorted Set (score=0, member=hex_hash)
//! nova:bot:{hex_addr}:{day}   →  Sayı (INCR)
//! ```
//!
//! ### TTL'ler
//! - Commit girişi: 60 saniye (slot süresi + güvenlik marjı)
//! - Slot set'i: 60 saniye
//! - Bot counter: 172800 saniye (48 saat — günlük sıfırlama için yeterli)
//!
//! ## Slot Hesabı
//! ```text
//! slot_id = (now_ms - genesis_time_ms) / SLOT_DURATION_MS
//! ```

use redis::aio::ConnectionManager;
use redis::AsyncCommands;

use nova_dex::mev::{SLOT_DURATION_MS, SwapIntent};
use nova_dex::types::{Address, Amount, AssetId, PoolId, Timestamp};

use crate::error::ApiError;

// ─── TTL SABİTLERİ ───────────────────────────────────────────────────────────

const COMMIT_TTL_SECS: u64 = 60;
const BOT_COUNTER_TTL_SECS: u64 = 172_800; // 48 saat

// ─── SLOT HESABI ─────────────────────────────────────────────────────────────

/// Verilen genesis ve şimdiki zamana göre mevcut slot ID'yi hesapla.
pub fn current_slot(genesis_time_ms: u64, now_ms: u64) -> u64 {
    now_ms.saturating_sub(genesis_time_ms) / SLOT_DURATION_MS
}

// ─── ANAHTAR OLUŞTURUCULARI ───────────────────────────────────────────────────

fn commit_key(hash_hex: &str) -> String {
    format!("nova:mev:commit:{hash_hex}")
}

fn intent_key(hash_hex: &str) -> String {
    format!("nova:mev:intent:{hash_hex}")
}

fn slot_key(slot_id: u64) -> String {
    format!("nova:mev:slot:{slot_id}")
}

fn bot_key(addr_hex: &str, day_epoch: u64) -> String {
    format!("nova:bot:{addr_hex}:{day_epoch}")
}

// ─── COMMIT İŞLEMLERİ ────────────────────────────────────────────────────────

/// Commit girdisi (Redis'e JSON olarak yazılır).
#[derive(serde::Serialize, serde::Deserialize)]
struct CommitMeta {
    sender_hex:    String,
    slot_id:       u64,
    committed_at_ms: u64,
    revealed:      bool,
}

/// Yeni commit gönder.
///
/// Aynı hash ikinci kez gönderilemez → `CommitAlreadyExists` döner.
pub async fn submit_commit(
    redis: &mut ConnectionManager,
    commit_hash: [u8; 32],
    sender: Address,
    genesis_time_ms: u64,
    now_ms: u64,
) -> Result<u64, ApiError> {
    let hash_hex  = hex::encode(commit_hash);
    let ck        = commit_key(&hash_hex);
    let slot_id   = current_slot(genesis_time_ms, now_ms);

    // Zaten varsa reddet
    let exists: bool = redis.exists(&ck).await.map_err(redis_err)?;
    if exists {
        return Err(ApiError::from(nova_dex::DexError::CommitAlreadyExists));
    }

    // Commit meta verisini kaydet
    let meta = CommitMeta {
        sender_hex:      hex::encode(sender.0),
        slot_id,
        committed_at_ms: now_ms,
        revealed:        false,
    };
    let json = serde_json::to_string(&meta).expect("CommitMeta serialize");

    // NX (Not eXists) ile set: race condition'a karşı ekstra koruma
    let ok: Option<String> = redis
        .set_options(
            &ck,
            &json,
            redis::SetOptions::default()
                .with_expiration(redis::SetExpiry::EX(COMMIT_TTL_SECS))
                .conditional_set(redis::ExistenceCheck::NX),
        )
        .await
        .map_err(redis_err)?;

    if ok.is_none() {
        // Başka bir instance aynı anda ekledi
        return Err(ApiError::from(nova_dex::DexError::CommitAlreadyExists));
    }

    // Slot sorted set'e ekle (lexicografik sıralama için score=0)
    let sk = slot_key(slot_id);
    let _: () = redis
        .zadd(&sk, &hash_hex, 0i64)
        .await
        .map_err(redis_err)?;
    let _: () = redis
        .expire(&sk, COMMIT_TTL_SECS as i64)
        .await
        .map_err(redis_err)?;

    Ok(slot_id)
}

/// Swap intent'i açıkla (reveal).
///
/// Hash doğrulaması başarısız olursa `RevealMismatch` döner.
/// Slot henüz kapanmamışsa `SlotStillOpen` döner.
pub async fn reveal_intent(
    redis: &mut ConnectionManager,
    intent: SwapIntent,
    genesis_time_ms: u64,
    now_ms: u64,
) -> Result<(), ApiError> {
    let hash = intent.commitment_hash();
    let hash_hex = hex::encode(hash);
    let ck = commit_key(&hash_hex);

    // Commit meta verisini oku
    let raw: Option<String> = redis.get(&ck).await.map_err(redis_err)?;
    let raw = raw.ok_or_else(|| ApiError::from(nova_dex::DexError::CommitNotFound))?;

    let meta: CommitMeta = serde_json::from_str(&raw)
        .map_err(|_| ApiError::InvalidInput("Commit meta parse hatasi".into()))?;

    // Zaten açıklandıysa reddet
    if meta.revealed {
        return Err(ApiError::from(nova_dex::DexError::AlreadyRevealed));
    }

    // Slot kapanmış mı?
    let current = current_slot(genesis_time_ms, now_ms);
    if current <= meta.slot_id {
        return Err(ApiError::from(nova_dex::DexError::SlotStillOpen));
    }

    // Sender doğrulama
    let sender_hex = hex::encode(intent.sender.0);
    if sender_hex != meta.sender_hex {
        return Err(ApiError::from(nova_dex::DexError::RevealMismatch));
    }

    // Commit meta'sını "revealed" olarak güncelle
    let updated = CommitMeta { revealed: true, ..meta };
    let json = serde_json::to_string(&updated).expect("CommitMeta serialize");
    let _: () = redis
        .set_ex(&ck, &json, COMMIT_TTL_SECS)
        .await
        .map_err(redis_err)?;

    // Intent'i ayrı anahtara yaz (hash sıralı erişim için)
    let ik = intent_key(&hash_hex);
    let intent_json = intent_to_json(&intent);
    let _: () = redis
        .set_ex(&ik, &intent_json, COMMIT_TTL_SECS)
        .await
        .map_err(redis_err)?;

    Ok(())
}

/// Belirli bir slot'taki tüm açıklanmış intent'leri döner (commit hash sırasıyla).
pub async fn get_revealed_for_slot(
    redis: &mut ConnectionManager,
    slot_id: u64,
) -> Vec<nova_dex::mev::SwapIntent> {
    let sk = slot_key(slot_id);

    // Slot set'indeki tüm hash'leri lexicografik sırayla al
    let hashes: Vec<String> = match redis.zrange(&sk, 0isize, -1isize).await {
        Ok(v)  => v,
        Err(e) => {
            tracing::warn!("Redis slot sorgusu basarisiz: {e}");
            return vec![];
        }
    };

    let mut intents = Vec::new();
    for hash_hex in &hashes {
        let ck = commit_key(hash_hex);
        let raw: Option<String> = match redis.get(&ck).await {
            Ok(v)  => v,
            Err(_) => continue,
        };
        let Some(raw) = raw else { continue };

        let meta: CommitMeta = match serde_json::from_str(&raw) {
            Ok(m)  => m,
            Err(_) => continue,
        };

        if !meta.revealed { continue; }

        let ik = intent_key(hash_hex);
        let intent_raw: Option<String> = match redis.get(&ik).await {
            Ok(v)  => v,
            Err(_) => continue,
        };
        let Some(intent_raw) = intent_raw else { continue };

        if let Some(intent) = json_to_intent(&intent_raw) {
            intents.push(intent);
        }
    }

    intents
}

// ─── BOT COUNTER ─────────────────────────────────────────────────────────────

/// Günlük swap sayacını artır ve yeni değeri döner.
///
/// Anahtar 48 saat TTL alır; UTC günlük sıfırlama gün epoch'uyla otomatik sağlanır.
pub async fn incr_bot_counter(
    redis: &mut ConnectionManager,
    address: &Address,
    now_secs: Timestamp,
) -> Result<i64, ApiError> {
    let day_epoch = now_secs / 86_400;
    let addr_hex  = hex::encode(address.0);
    let key       = bot_key(&addr_hex, day_epoch);

    let count: i64 = redis.incr(&key, 1i64).await.map_err(redis_err)?;

    // İlk kez oluşturulduğunda TTL set et (INCR sonrası 1 ise yeni anahtar)
    if count == 1 {
        let _: () = redis
            .expire(&key, BOT_COUNTER_TTL_SECS as i64)
            .await
            .map_err(redis_err)?;
    }

    Ok(count)
}

// ─── JSON YARDIMCILARI ────────────────────────────────────────────────────────

/// SwapIntent → JSON dizi.
fn intent_to_json(i: &SwapIntent) -> String {
    serde_json::json!({
        "sender":         hex::encode(i.sender.0),
        "pool_id":        hex::encode(i.pool_id.0),
        "asset_in":       hex::encode(i.asset_in.0),
        "asset_out":      hex::encode(i.asset_out.0),
        "amount_in":      i.amount_in.to_string(),
        "min_amount_out": i.min_amount_out.to_string(),
        "nonce":          hex::encode(i.nonce),
    })
    .to_string()
}

/// JSON dizi → SwapIntent (None: parse hatası).
fn json_to_intent(raw: &str) -> Option<SwapIntent> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;

    fn parse_hex32(v: &serde_json::Value, key: &str) -> Option<[u8; 32]> {
        let s = v[key].as_str()?;
        let bytes = hex::decode(s).ok()?;
        bytes.try_into().ok()
    }

    fn parse_u128(v: &serde_json::Value, key: &str) -> Option<Amount> {
        v[key].as_str()?.parse().ok()
    }

    let nonce_bytes = parse_hex32(&v, "nonce")?;

    Some(SwapIntent {
        sender:         Address(parse_hex32(&v, "sender")?),
        pool_id:        PoolId(parse_hex32(&v, "pool_id")?),
        asset_in:       AssetId(parse_hex32(&v, "asset_in")?),
        asset_out:      AssetId(parse_hex32(&v, "asset_out")?),
        amount_in:      parse_u128(&v, "amount_in")?,
        min_amount_out: parse_u128(&v, "min_amount_out")?,
        nonce:          nonce_bytes,
    })
}

// ─── HATA DÖNÜŞÜMÜ ───────────────────────────────────────────────────────────

fn redis_err(e: redis::RedisError) -> ApiError {
    tracing::error!("Redis hatasi: {e}");
    ApiError::InvalidInput(format!("Önbellek hatasi: {e}"))
}

