use axum::{
    extract::{Path, State},
    Json,
};
use nova_dex::amm;
use nova_dex::liquidity::LockInfo;
use nova_dex::types::{Address, PoolId};
use serde_json::Value;

use crate::{
    auth,
    db::{self, bytes32, db_to_lock_duration, db_to_unlock_at,
         str_to_u128, u128_to_str},
    dto::{
        AddLiquidityRequest, AddLiquidityResponse, ExtendLockRequest,
        LpPositionResponse, RemoveLiquidityRequest, RemoveLiquidityResponse,
    },
    error::{now_secs, parse_hex32, parse_u128, ApiError},
    state::AppState,
};

pub async fn add_liquidity(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<AddLiquidityRequest>,
) -> Result<Json<AddLiquidityResponse>, ApiError> {
    let pool_id  = PoolId(parse_hex32(&id)?);
    let provider = Address(parse_hex32(&body.provider)?);
    let amount_a = parse_u128(&body.amount_a)?;
    let amount_b = parse_u128(&body.amount_b)?;
    let now      = now_secs();

    if amount_a == 0 || amount_b == 0 {
        return Err(ApiError::from(nova_dex::DexError::ZeroAmount));
    }

    // AUTH-001
    auth::verify_add_liquidity(
        &pool_id, &provider,
        &body.public_key, &body.signature,
        amount_a, amount_b,
        body.timestamp_secs, now,
    )?;

    let mut tx = state.db.begin().await.map_err(db::db_err)?;

    // Pool'u kilitle ve oku
    let pool_row = db::get_pool_for_update(&mut tx, &pool_id)
        .await?
        .ok_or_else(|| ApiError::from(nova_dex::DexError::PoolNotFound))?;

    let reserve_a = str_to_u128(&pool_row.reserve_a);
    let reserve_b = str_to_u128(&pool_row.reserve_b);
    let total_lp  = str_to_u128(&pool_row.total_lp);

    // Mevcut orana normalize et
    let (actual_a, actual_b) = amm::normalize_liquidity(
        amount_a, amount_b, reserve_a, reserve_b, total_lp,
    ).map_err(ApiError::from)?;

    // LP token hesabı
    let lp_tokens = amm::compute_lp_tokens_to_mint(
        actual_a, actual_b, reserve_a, reserve_b, total_lp,
    ).map_err(ApiError::from)?;

    // Mevcut LP pozisyonu oku (ya da sıfır kabul et)
    let existing_lp = db::get_lp_for_update(&mut tx, &pool_id, &provider)
        .await?
        .map(|r| {
            let lp  = str_to_u128(&r.lp_tokens);
            let da  = str_to_u128(&r.deposited_a);
            let db_ = str_to_u128(&r.deposited_b);
            (lp, da, db_)
        })
        .unwrap_or((0, 0, 0));

    let new_lp  = existing_lp.0.saturating_add(lp_tokens);
    let new_da  = existing_lp.1.saturating_add(actual_a);
    let new_db_ = existing_lp.2.saturating_add(actual_b);

    // Pool rezervlerini güncelle
    let new_reserve_a = reserve_a.saturating_add(actual_a);
    let new_reserve_b = reserve_b.saturating_add(actual_b);
    let new_total_lp  = total_lp.saturating_add(lp_tokens);

    db::update_pool_after_swap(
        &mut tx, &pool_id,
        new_reserve_a, new_reserve_b,
        str_to_u128(&pool_row.total_fees_collected),
        str_to_u128(&pool_row.total_volume_a),
        pool_row.swap_count as u64,
    ).await?;

    // total_lp'yi de güncelle — ayrı sorgu
    sqlx::query("UPDATE pools SET total_lp = $1 WHERE id = $2")
        .bind(u128_to_str(new_total_lp))
        .bind(pool_id.0.as_slice())
        .execute(&mut *tx)
        .await
        .map_err(db::db_err)?;

    // LP pozisyonu kaydet
    db::upsert_lp_position(
        &mut tx, &pool_id, &provider,
        new_lp, new_da, new_db_, now,
    ).await?;

    tx.commit().await.map_err(db::db_err)?;

    Ok(Json(AddLiquidityResponse {
        lp_tokens: lp_tokens.to_string(),
        actual_a:  actual_a.to_string(),
        actual_b:  actual_b.to_string(),
    }))
}

pub async fn remove_liquidity(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<RemoveLiquidityRequest>,
) -> Result<Json<RemoveLiquidityResponse>, ApiError> {
    let pool_id    = PoolId(parse_hex32(&id)?);
    let provider   = Address(parse_hex32(&body.provider)?);
    let lp_to_burn = parse_u128(&body.lp_to_burn)?;
    let now        = now_secs();

    // AUTH-001
    auth::verify_remove_liquidity(
        &pool_id, &provider,
        &body.public_key, &body.signature,
        lp_to_burn,
        body.timestamp_secs, now,
    )?;

    let mut tx = state.db.begin().await.map_err(db::db_err)?;

    // Pool'u kilitle
    let pool_row = db::get_pool_for_update(&mut tx, &pool_id)
        .await?
        .ok_or_else(|| ApiError::from(nova_dex::DexError::PoolNotFound))?;

    // Kilit kontrolü
    let lock_duration = db_to_lock_duration(pool_row.lock_duration_secs);
    let lock_info = LockInfo {
        creator:   nova_dex::types::Address(bytes32(&pool_row.lock_creator)),
        duration:  lock_duration,
        locked_at: pool_row.lock_locked_at as u64,
        unlock_at: db_to_unlock_at(pool_row.lock_unlock_at),
    };
    lock_info.assert_withdrawable(now).map_err(ApiError::from)?;

    // LP pozisyonu oku
    let lp_row = db::get_lp_for_update(&mut tx, &pool_id, &provider)
        .await?
        .ok_or_else(|| ApiError::from(nova_dex::DexError::LiquidityProviderNotFound))?;

    let current_lp = str_to_u128(&lp_row.lp_tokens);
    if current_lp < lp_to_burn {
        return Err(ApiError::from(nova_dex::DexError::InsufficientBalance {
            available: current_lp,
            requested: lp_to_burn,
        }));
    }

    let reserve_a = str_to_u128(&pool_row.reserve_a);
    let reserve_b = str_to_u128(&pool_row.reserve_b);
    let total_lp  = str_to_u128(&pool_row.total_lp);

    // Çekilecek miktarları hesapla
    let (amount_a, amount_b) = amm::compute_withdrawal_amounts(
        lp_to_burn, total_lp, reserve_a, reserve_b,
    ).map_err(ApiError::from)?;

    // Pool rezervlerini güncelle
    let new_reserve_a = reserve_a.saturating_sub(amount_a);
    let new_reserve_b = reserve_b.saturating_sub(amount_b);
    let new_total_lp  = total_lp.saturating_sub(lp_to_burn);

    db::update_pool_after_swap(
        &mut tx, &pool_id,
        new_reserve_a, new_reserve_b,
        str_to_u128(&pool_row.total_fees_collected),
        str_to_u128(&pool_row.total_volume_a),
        pool_row.swap_count as u64,
    ).await?;

    sqlx::query("UPDATE pools SET total_lp = $1 WHERE id = $2")
        .bind(u128_to_str(new_total_lp))
        .bind(pool_id.0.as_slice())
        .execute(&mut *tx)
        .await
        .map_err(db::db_err)?;

    // LP pozisyonu güncelle
    db::update_lp_after_remove(&mut tx, &pool_id, &provider, current_lp - lp_to_burn).await?;

    tx.commit().await.map_err(db::db_err)?;

    Ok(Json(RemoveLiquidityResponse {
        amount_a: amount_a.to_string(),
        amount_b: amount_b.to_string(),
    }))
}

pub async fn extend_lock(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ExtendLockRequest>,
) -> Result<Json<Value>, ApiError> {
    let pool_id  = PoolId(parse_hex32(&id)?);
    let caller   = Address(parse_hex32(&body.caller)?);
    let new_dur: nova_dex::types::LockDuration = body.new_duration.into();
    let now      = now_secs();

    // AUTH-001
    auth::verify_extend_lock(
        &pool_id, &caller,
        &body.public_key, &body.signature,
        new_dur,
        body.timestamp_secs, now,
    )?;

    let mut tx = state.db.begin().await.map_err(db::db_err)?;

    let pool_row = db::get_pool_for_update(&mut tx, &pool_id)
        .await?
        .ok_or_else(|| ApiError::from(nova_dex::DexError::PoolNotFound))?;

    // Yalnızca creator yapabilir
    if bytes32(&pool_row.creator) != caller.0 {
        return Err(ApiError::from(nova_dex::DexError::Unauthorized));
    }

    // LockInfo ile extend doğrulaması
    let lock_duration = db_to_lock_duration(pool_row.lock_duration_secs);
    let mut lock_info = LockInfo {
        creator:   nova_dex::types::Address(bytes32(&pool_row.lock_creator)),
        duration:  lock_duration,
        locked_at: pool_row.lock_locked_at as u64,
        unlock_at: db_to_unlock_at(pool_row.lock_unlock_at),
    };

    lock_info.extend(new_dur, now).map_err(ApiError::from)?;

    db::update_pool_lock(&mut tx, &pool_id, lock_info.duration, lock_info.unlock_at).await?;

    tx.commit().await.map_err(db::db_err)?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn get_lp_position(
    State(state): State<AppState>,
    Path((id, address)): Path<(String, String)>,
) -> Result<Json<LpPositionResponse>, ApiError> {
    let pool_id  = PoolId(parse_hex32(&id)?);
    let provider = Address(parse_hex32(&address)?);

    let row = db::get_lp_position(&state.db, &pool_id, &provider)
        .await?
        .ok_or_else(|| ApiError::from(nova_dex::DexError::LiquidityProviderNotFound))?;

    Ok(Json(LpPositionResponse {
        provider:    hex::encode(&row.provider),
        lp_tokens:   row.lp_tokens,
        deposited_a: row.deposited_a,
        deposited_b: row.deposited_b,
        created_at:  row.created_at as u64,
    }))
}
