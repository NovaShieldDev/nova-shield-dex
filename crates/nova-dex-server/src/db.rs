//! Nova DEX — PostgreSQL Veri Katmanı.
//!
//! ## Tasarım Prensipleri
//!
//! - **Satır düzeyi kilit**: Yazma işlemleri `SELECT ... FOR UPDATE` ile yalnızca
//!   ilgili pool satırını kilitler; farklı pool'lardaki işlemler paralel çalışır.
//! - **Miktarlar TEXT olarak saklanır**: u128 PostgreSQL NUMERIC yerine VARCHAR(40)
//!   ondalık dizgisi olarak tutulur; aritmetik Rust tarafında yapılır.
//! - **Zaman damgaları BIGINT**: u64 timestamp'ler için yeterlidir (2554 yılına kadar).
//! - **Kilit süresi**: lock_duration_secs = 0 → Unlimited; diğerleri saniye.
//! - **lock_unlock_at**: Unlimited → i64::MAX; diğerleri hesaplanan epoch.

use sqlx::PgPool;
use sqlx::Row;

use nova_dex::liquidity::LockInfo;
use nova_dex::types::{
    Address, Amount, AssetId, LockDuration, PoolId, Timestamp,
    PAYOUT_CYCLE_SECS, PAYOUT_TRANSFER_FEE_NNOVA,
};
use nova_dex::creator_payout::PayoutRecord;

use crate::error::ApiError;

// ─── TİP DÖNÜŞÜM YARDIMCILARI ────────────────────────────────────────────────

/// Vec<u8> → [u8; 32] (DB'den gelen BYTEA).
pub fn bytes32(v: &[u8]) -> [u8; 32] {
    let mut arr = [0u8; 32];
    let len = v.len().min(32);
    arr[..len].copy_from_slice(&v[..len]);
    arr
}

/// u128 → ondalık dizi (VARCHAR(40) olarak saklanır).
pub fn u128_to_str(v: u128) -> String {
    v.to_string()
}

/// Ondalık dizi → u128 (DB'den gelen VARCHAR).
pub fn str_to_u128(s: &str) -> u128 {
    s.parse().unwrap_or(0)
}

/// LockDuration → DB BIGINT (0 = Unlimited).
pub fn lock_duration_to_db(d: LockDuration) -> i64 {
    if d.is_unlimited() { 0 } else { d.to_seconds() as i64 }
}

/// DB BIGINT → LockDuration.
pub fn db_to_lock_duration(secs: i64) -> LockDuration {
    match secs {
        0             => LockDuration::Unlimited,
        86_400        => LockDuration::OneDay,
        259_200       => LockDuration::ThreeDays,
        604_800       => LockDuration::OneWeek,
        2_592_000     => LockDuration::OneMonth,
        31_536_000    => LockDuration::TwelveMonths,
        _             => LockDuration::Unlimited, // bilinmeyen → güvenli default
    }
}

/// lock_unlock_at u64 → DB i64 (Unlimited: i64::MAX).
pub fn unlock_at_to_db(unlock_at: u64) -> i64 {
    if unlock_at == u64::MAX { i64::MAX } else { unlock_at as i64 }
}

/// DB i64 → lock_unlock_at u64 (i64::MAX → u64::MAX).
pub fn db_to_unlock_at(v: i64) -> u64 {
    if v == i64::MAX { u64::MAX } else { v as u64 }
}

/// Pool kilit durumu dizi gösterimi.
pub fn lock_status_from_db(lock_duration_secs: i64, lock_unlock_at: i64, now: u64) -> String {
    if lock_duration_secs == 0 {
        return "Permanent Liquidity ♾".to_string();
    }
    let unlock_at = db_to_unlock_at(lock_unlock_at);
    if now >= unlock_at {
        "Kilit Açık".to_string()
    } else {
        let remaining = unlock_at.saturating_sub(now);
        let days  = remaining / 86_400;
        let hours = (remaining % 86_400) / 3_600;
        let mins  = (remaining % 3_600) / 60;
        format!("Kilitli — {days}g {hours}s {mins}dk kaldı")
    }
}

// ─── VERİTABANI ŞEMASI KURULUMU ──────────────────────────────────────────────

/// Tüm tabloları yoksa oluştur ve başlangıç verilerini ekle.
///
/// Uygulama başlarken bir kez çağrılır.
pub async fn setup_schema(db: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS pools (
            id                   BYTEA        PRIMARY KEY,
            asset_a              BYTEA        NOT NULL,
            asset_b              BYTEA        NOT NULL,
            creator              BYTEA        NOT NULL,
            reserve_a            VARCHAR(40)  NOT NULL,
            reserve_b            VARCHAR(40)  NOT NULL,
            total_lp             VARCHAR(40)  NOT NULL,
            lock_creator         BYTEA        NOT NULL,
            lock_duration_secs   BIGINT       NOT NULL,
            lock_locked_at       BIGINT       NOT NULL,
            lock_unlock_at       BIGINT       NOT NULL,
            created_at           BIGINT       NOT NULL,
            total_volume_a       VARCHAR(40)  NOT NULL DEFAULT '0',
            total_fees_collected VARCHAR(40)  NOT NULL DEFAULT '0',
            swap_count           BIGINT       NOT NULL DEFAULT 0
        )"
    ).execute(db).await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS lp_positions (
            pool_id     BYTEA        NOT NULL,
            provider    BYTEA        NOT NULL,
            lp_tokens   VARCHAR(40)  NOT NULL,
            deposited_a VARCHAR(40)  NOT NULL,
            deposited_b VARCHAR(40)  NOT NULL,
            created_at  BIGINT       NOT NULL,
            PRIMARY KEY (pool_id, provider)
        )"
    ).execute(db).await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS registry_meta (
            key   TEXT        PRIMARY KEY,
            value VARCHAR(40) NOT NULL DEFAULT '0'
        )"
    ).execute(db).await?;

    // Başlangıç meta değerleri
    sqlx::query(
        "INSERT INTO registry_meta (key, value) VALUES
            ('treasury_balance', '0'),
            ('genesis_time', '0'),
            ('last_payout_at', '0')
         ON CONFLICT (key) DO NOTHING"
    ).execute(db).await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS pending_payouts (
            creator  BYTEA       PRIMARY KEY,
            amount   VARCHAR(40) NOT NULL DEFAULT '0'
        )"
    ).execute(db).await?;

    Ok(())
}

// ─── GENESIS ZAMANI ───────────────────────────────────────────────────────────

/// Genesis zamanını oku veya ilk kez set et (Unix saniye).
///
/// Sunucu ilk başladığında set edilir; sonraki başlatmalarda mevcut değer korunur.
/// MEV slot hesabı için tüm instance'larda aynı genesis_time kullanılmalıdır.
pub async fn get_or_init_genesis_time(db: &PgPool, now_secs: u64) -> Result<u64, ApiError> {
    // Genesis 0 ise ilk başlatma — şimdiki zamanı yaz
    let row = sqlx::query("SELECT value FROM registry_meta WHERE key = 'genesis_time'")
        .fetch_one(db)
        .await
        .map_err(db_err)?;

    let current: String = row.try_get("value").map_err(db_err)?;
    let genesis: u64 = current.parse().unwrap_or(0);

    if genesis == 0 {
        let val = u128_to_str(now_secs as u128);
        sqlx::query("UPDATE registry_meta SET value = $1 WHERE key = 'genesis_time'")
            .bind(&val)
            .execute(db)
            .await
            .map_err(db_err)?;
        Ok(now_secs)
    } else {
        Ok(genesis)
    }
}

// ─── POOL İŞLEMLERİ ──────────────────────────────────────────────────────────

/// Pool satırı yapısı (DB'den çekilen ham veri).
pub struct PoolRow {
    pub id:                   Vec<u8>,
    pub asset_a:              Vec<u8>,
    pub asset_b:              Vec<u8>,
    pub creator:              Vec<u8>,
    pub reserve_a:            String,
    pub reserve_b:            String,
    pub total_lp:             String,
    pub lock_creator:         Vec<u8>,
    pub lock_duration_secs:   i64,
    pub lock_locked_at:       i64,
    pub lock_unlock_at:       i64,
    pub created_at:           i64,
    pub total_volume_a:       String,
    pub total_fees_collected: String,
    pub swap_count:           i64,
}

/// sqlx Row → PoolRow dönüşümü.
fn row_to_pool_row(row: &sqlx::postgres::PgRow) -> Result<PoolRow, sqlx::Error> {
    Ok(PoolRow {
        id:                   row.try_get("id")?,
        asset_a:              row.try_get("asset_a")?,
        asset_b:              row.try_get("asset_b")?,
        creator:              row.try_get("creator")?,
        reserve_a:            row.try_get("reserve_a")?,
        reserve_b:            row.try_get("reserve_b")?,
        total_lp:             row.try_get("total_lp")?,
        lock_creator:         row.try_get("lock_creator")?,
        lock_duration_secs:   row.try_get("lock_duration_secs")?,
        lock_locked_at:       row.try_get("lock_locked_at")?,
        lock_unlock_at:       row.try_get("lock_unlock_at")?,
        created_at:           row.try_get("created_at")?,
        total_volume_a:       row.try_get("total_volume_a")?,
        total_fees_collected: row.try_get("total_fees_collected")?,
        swap_count:           row.try_get("swap_count")?,
    })
}

const POOL_SELECT: &str =
    "SELECT id, asset_a, asset_b, creator, reserve_a, reserve_b, total_lp,
            lock_creator, lock_duration_secs, lock_locked_at, lock_unlock_at,
            created_at, total_volume_a, total_fees_collected, swap_count
     FROM pools";

/// Tüm pool'ları oku (sıralama: Permanent önce, ardından hacme göre azalan).
pub async fn list_pools(db: &PgPool) -> Result<Vec<PoolRow>, ApiError> {
    let rows = sqlx::query(&format!(
        "{POOL_SELECT} ORDER BY lock_duration_secs ASC, total_volume_a DESC"
    ))
    .fetch_all(db)
    .await
    .map_err(db_err)?;

    rows.iter().map(row_to_pool_row).map(|r| r.map_err(db_err)).collect()
}

/// Tek pool oku (kilit yok, salt okunur).
pub async fn get_pool(db: &PgPool, pool_id: &PoolId) -> Result<Option<PoolRow>, ApiError> {
    let row = sqlx::query(&format!("{POOL_SELECT} WHERE id = $1"))
        .bind(pool_id.0.as_slice())
        .fetch_optional(db)
        .await
        .map_err(db_err)?;

    row.map(|r| row_to_pool_row(&r).map_err(db_err)).transpose()
}

/// Pool oku ve satır kilidi al (transaction içinde kullanılır).
pub async fn get_pool_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    pool_id: &PoolId,
) -> Result<Option<PoolRow>, ApiError> {
    let row = sqlx::query(&format!("{POOL_SELECT} WHERE id = $1 FOR UPDATE"))
        .bind(pool_id.0.as_slice())
        .fetch_optional(&mut **tx)
        .await
        .map_err(db_err)?;

    row.map(|r| row_to_pool_row(&r).map_err(db_err)).transpose()
}

/// Yeni pool oluştur.
#[allow(clippy::too_many_arguments)]
pub async fn insert_pool(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    pool_id: &PoolId,
    asset_a: &AssetId,
    asset_b: &AssetId,
    creator: &Address,
    reserve_a: Amount,
    reserve_b: Amount,
    total_lp: Amount,
    lock_duration: LockDuration,
    now: Timestamp,
) -> Result<(), ApiError> {
    let lock_info = LockInfo::new(*creator, lock_duration, now);

    sqlx::query(
        "INSERT INTO pools (
            id, asset_a, asset_b, creator, reserve_a, reserve_b, total_lp,
            lock_creator, lock_duration_secs, lock_locked_at, lock_unlock_at,
            created_at, total_volume_a, total_fees_collected, swap_count
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'0','0',0)"
    )
    .bind(pool_id.0.as_slice())
    .bind(asset_a.0.as_slice())
    .bind(asset_b.0.as_slice())
    .bind(creator.0.as_slice())
    .bind(u128_to_str(reserve_a))
    .bind(u128_to_str(reserve_b))
    .bind(u128_to_str(total_lp))
    .bind(creator.0.as_slice())
    .bind(lock_duration_to_db(lock_duration))
    .bind(now as i64)
    .bind(unlock_at_to_db(lock_info.unlock_at))
    .bind(now as i64)
    .execute(&mut **tx)
    .await
    .map_err(db_err)?;

    Ok(())
}

/// Pool rezervlerini ve istatistiklerini swap sonrası güncelle.
#[allow(clippy::too_many_arguments)]
pub async fn update_pool_after_swap(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    pool_id: &PoolId,
    new_reserve_a: Amount,
    new_reserve_b: Amount,
    new_total_fees: Amount,
    new_volume_a: Amount,
    new_swap_count: u64,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE pools SET
            reserve_a            = $1,
            reserve_b            = $2,
            total_fees_collected = $3,
            total_volume_a       = $4,
            swap_count           = $5
         WHERE id = $6"
    )
    .bind(u128_to_str(new_reserve_a))
    .bind(u128_to_str(new_reserve_b))
    .bind(u128_to_str(new_total_fees))
    .bind(u128_to_str(new_volume_a))
    .bind(new_swap_count as i64)
    .bind(pool_id.0.as_slice())
    .execute(&mut **tx)
    .await
    .map_err(db_err)?;

    Ok(())
}

/// Pool kilit süresini güncelle (extend_lock).
pub async fn update_pool_lock(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    pool_id: &PoolId,
    new_duration: LockDuration,
    new_unlock_at: Timestamp,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE pools SET lock_duration_secs = $1, lock_unlock_at = $2 WHERE id = $3"
    )
    .bind(lock_duration_to_db(new_duration))
    .bind(unlock_at_to_db(new_unlock_at))
    .bind(pool_id.0.as_slice())
    .execute(&mut **tx)
    .await
    .map_err(db_err)?;

    Ok(())
}

// ─── LP POZİSYON İŞLEMLERİ ───────────────────────────────────────────────────

/// LP pozisyon satırı.
pub struct LpRow {
    pub provider:    Vec<u8>,
    pub lp_tokens:   String,
    pub deposited_a: String,
    pub deposited_b: String,
    pub created_at:  i64,
}

/// LP pozisyon oku (salt okunur).
pub async fn get_lp_position(
    db: &PgPool,
    pool_id: &PoolId,
    provider: &Address,
) -> Result<Option<LpRow>, ApiError> {
    let row = sqlx::query(
        "SELECT provider, lp_tokens, deposited_a, deposited_b, created_at
         FROM lp_positions WHERE pool_id = $1 AND provider = $2"
    )
    .bind(pool_id.0.as_slice())
    .bind(provider.0.as_slice())
    .fetch_optional(db)
    .await
    .map_err(db_err)?;

    row.map(|r| -> Result<LpRow, sqlx::Error> {
        Ok(LpRow {
            provider:    r.try_get("provider")?,
            lp_tokens:   r.try_get("lp_tokens")?,
            deposited_a: r.try_get("deposited_a")?,
            deposited_b: r.try_get("deposited_b")?,
            created_at:  r.try_get("created_at")?,
        })
    })
    .transpose()
    .map_err(db_err)
}

/// LP pozisyon oku ve kilitle (transaction içinde).
pub async fn get_lp_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    pool_id: &PoolId,
    provider: &Address,
) -> Result<Option<LpRow>, ApiError> {
    let row = sqlx::query(
        "SELECT provider, lp_tokens, deposited_a, deposited_b, created_at
         FROM lp_positions WHERE pool_id = $1 AND provider = $2 FOR UPDATE"
    )
    .bind(pool_id.0.as_slice())
    .bind(provider.0.as_slice())
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_err)?;

    row.map(|r| -> Result<LpRow, sqlx::Error> {
        Ok(LpRow {
            provider:    r.try_get("provider")?,
            lp_tokens:   r.try_get("lp_tokens")?,
            deposited_a: r.try_get("deposited_a")?,
            deposited_b: r.try_get("deposited_b")?,
            created_at:  r.try_get("created_at")?,
        })
    })
    .transpose()
    .map_err(db_err)
}

/// LP pozisyon ekle veya güncelle (add_liquidity).
pub async fn upsert_lp_position(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    pool_id: &PoolId,
    provider: &Address,
    new_lp: Amount,
    new_dep_a: Amount,
    new_dep_b: Amount,
    now: Timestamp,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO lp_positions (pool_id, provider, lp_tokens, deposited_a, deposited_b, created_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (pool_id, provider) DO UPDATE SET
            lp_tokens   = excluded.lp_tokens,
            deposited_a = excluded.deposited_a,
            deposited_b = excluded.deposited_b"
    )
    .bind(pool_id.0.as_slice())
    .bind(provider.0.as_slice())
    .bind(u128_to_str(new_lp))
    .bind(u128_to_str(new_dep_a))
    .bind(u128_to_str(new_dep_b))
    .bind(now as i64)
    .execute(&mut **tx)
    .await
    .map_err(db_err)?;

    Ok(())
}

/// LP pozisyon güncelle (remove_liquidity).
pub async fn update_lp_after_remove(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    pool_id: &PoolId,
    provider: &Address,
    new_lp: Amount,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE lp_positions SET lp_tokens = $1 WHERE pool_id = $2 AND provider = $3"
    )
    .bind(u128_to_str(new_lp))
    .bind(pool_id.0.as_slice())
    .bind(provider.0.as_slice())
    .execute(&mut **tx)
    .await
    .map_err(db_err)?;

    Ok(())
}

// ─── TREASURY & CREATOR FEE ───────────────────────────────────────────────────

/// Treasury bakiyesini oku ve kilitle (transaction içinde).
pub async fn get_treasury_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<Amount, ApiError> {
    let row = sqlx::query(
        "SELECT value FROM registry_meta WHERE key = 'treasury_balance' FOR UPDATE"
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(db_err)?;

    let v: String = row.try_get("value").map_err(db_err)?;
    Ok(str_to_u128(&v))
}

/// Treasury bakiyesini yaz (transaction içinde, get_treasury_for_update sonrası).
pub async fn set_treasury(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    amount: Amount,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE registry_meta SET value = $1 WHERE key = 'treasury_balance'")
        .bind(u128_to_str(amount))
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    Ok(())
}

/// Bekleyen creator fee'yi oku ve kilitle (transaction içinde).
pub async fn get_pending_payout_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    creator: &Address,
) -> Result<Amount, ApiError> {
    let row = sqlx::query(
        "SELECT amount FROM pending_payouts WHERE creator = $1 FOR UPDATE"
    )
    .bind(creator.0.as_slice())
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_err)?;

    match row {
        Some(r) => {
            let v: String = r.try_get("amount").map_err(db_err)?;
            Ok(str_to_u128(&v))
        }
        None => Ok(0),
    }
}

/// Creator fee bekleyen miktarı güncelle (transaction içinde).
pub async fn set_pending_payout(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    creator: &Address,
    amount: Amount,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO pending_payouts (creator, amount) VALUES ($1, $2)
         ON CONFLICT (creator) DO UPDATE SET amount = excluded.amount"
    )
    .bind(creator.0.as_slice())
    .bind(u128_to_str(amount))
    .execute(&mut **tx)
    .await
    .map_err(db_err)?;

    Ok(())
}

/// Treasury + creator fee'yi atomik olarak güncelle (tek transaction).
///
/// Swap'tan sonra çağrılır; pool transaction'dan ayrı tutulur.
pub async fn apply_fee_distribution(
    db: &PgPool,
    creator: &Address,
    creator_fee: Amount,
    treasury_fee: Amount,
) -> Result<(), ApiError> {
    let mut tx = db.begin().await.map_err(db_err)?;

    // Treasury
    let current_treasury = get_treasury_for_update(&mut tx).await?;
    set_treasury(&mut tx, current_treasury.saturating_add(treasury_fee)).await?;

    // Creator payout
    let current_creator = get_pending_payout_for_update(&mut tx, creator).await?;
    set_pending_payout(&mut tx, creator, current_creator.saturating_add(creator_fee)).await?;

    tx.commit().await.map_err(db_err)?;
    Ok(())
}

// ─── PAYOUT İŞLEMLERİ ────────────────────────────────────────────────────────

/// Bekleyen tüm creator ödemelerini işle (24 saatlik döngü).
///
/// - 24 saat dolmamışsa boş Vec döner.
/// - Transfer ücreti net miktardan düşülür.
/// - Küçük miktarlar (≤ transfer ücreti) sonraki döngüye taşınır.
pub async fn process_payouts(db: &PgPool, now: Timestamp) -> Result<Vec<PayoutRecord>, ApiError> {
    // last_payout_at kontrolü (kilit gerektirmiyor — tek arka plan görevi çağırır)
    let meta_row = sqlx::query(
        "SELECT value FROM registry_meta WHERE key = 'last_payout_at'"
    )
    .fetch_one(db)
    .await
    .map_err(db_err)?;

    let last: u64 = meta_row
        .try_get::<String, _>("value")
        .map_err(db_err)?
        .parse()
        .unwrap_or(0);

    if now < last.saturating_add(PAYOUT_CYCLE_SECS) {
        return Ok(vec![]);
    }

    // Tüm bekleyen ödemeleri çek
    let payout_rows = sqlx::query("SELECT creator, amount FROM pending_payouts WHERE amount != '0'")
        .fetch_all(db)
        .await
        .map_err(db_err)?;

    let mut records = Vec::new();

    for row in &payout_rows {
        let creator_bytes: Vec<u8> = row.try_get("creator").map_err(db_err)?;
        let amount_str: String     = row.try_get("amount").map_err(db_err)?;
        let gross = str_to_u128(&amount_str);

        if gross == 0 || gross <= PAYOUT_TRANSFER_FEE_NNOVA {
            continue;
        }

        let net = gross - PAYOUT_TRANSFER_FEE_NNOVA;
        records.push(PayoutRecord {
            creator:      Address(bytes32(&creator_bytes)),
            gross_amount: gross,
            transfer_fee: PAYOUT_TRANSFER_FEE_NNOVA,
            net_amount:   net,
            paid_at:      now,
        });
    }

    if records.is_empty() {
        // Hiçbir şey ödenmiyor; last_payout_at güncelle
        sqlx::query("UPDATE registry_meta SET value = $1 WHERE key = 'last_payout_at'")
            .bind(u128_to_str(now as u128))
            .execute(db)
            .await
            .map_err(db_err)?;
        return Ok(vec![]);
    }

    // Ödenen creator'ların bekleyen miktarlarını sıfırla
    let mut tx = db.begin().await.map_err(db_err)?;
    for rec in &records {
        sqlx::query("UPDATE pending_payouts SET amount = '0' WHERE creator = $1")
            .bind(rec.creator.0.as_slice())
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    sqlx::query("UPDATE registry_meta SET value = $1 WHERE key = 'last_payout_at'")
        .bind(u128_to_str(now as u128))
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;

    Ok(records)
}

// ─── POOL VARLIK KONTROLÜ ─────────────────────────────────────────────────────

/// Pool_id'ye göre pool var mı?
pub async fn pool_exists(db: &PgPool, pool_id: &PoolId) -> Result<bool, ApiError> {
    let row = sqlx::query("SELECT 1 FROM pools WHERE id = $1")
        .bind(pool_id.0.as_slice())
        .fetch_optional(db)
        .await
        .map_err(db_err)?;
    Ok(row.is_some())
}

/// Pool sayısını döner (/health endpoint için).
pub async fn pool_count(db: &PgPool) -> Result<usize, ApiError> {
    let row = sqlx::query("SELECT COUNT(*) AS cnt FROM pools")
        .fetch_one(db)
        .await
        .map_err(db_err)?;
    let cnt: i64 = row.try_get("cnt").map_err(db_err)?;
    Ok(cnt as usize)
}

// ─── HATA DÖNÜŞÜMÜ ───────────────────────────────────────────────────────────

/// sqlx hatalarını ApiError'a dönüştür.
pub fn db_err(e: sqlx::Error) -> ApiError {
    tracing::error!("DB hatasi: {e}");
    ApiError::InvalidInput(format!("Veritabani hatasi: {e}"))
}

