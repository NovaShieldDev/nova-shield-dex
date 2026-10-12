use axum::{extract::{Path, State}, Json};
use nova_dex::amm;
use nova_dex::pool::required_creation_fee_nnova;
use nova_dex::types::{Address, AssetId, PoolId};

use crate::{
    auth,
    db::{self, lock_status_from_db, db_to_lock_duration},
    dto::{CreatePoolRequest, CreatePoolResponse, LockDurationDto, PoolResponse},
    error::{now_secs, parse_hex32, parse_u128, ApiError},
    state::AppState,
};

pub async fn create_pool(
    State(state): State<AppState>,
    Json(body): Json<CreatePoolRequest>,
) -> Result<Json<CreatePoolResponse>, ApiError> {
    let creator      = Address(parse_hex32(&body.creator)?);
    let asset_a      = AssetId(parse_hex32(&body.asset_a)?);
    let asset_b      = AssetId(parse_hex32(&body.asset_b)?);
    let initial_a    = parse_u128(&body.initial_a)?;
    let initial_b    = parse_u128(&body.initial_b)?;
    let creation_fee = parse_u128(&body.creation_fee_paid)?;
    let lock         = body.lock_duration.into();
    let now          = now_secs();

    // AUTH-001: ECDSA imza doğrulaması
    auth::verify_create_pool(
        &creator, &asset_a, &asset_b,
        &body.public_key, &body.signature,
        initial_a, initial_b, creation_fee,
        body.timestamp_secs, now,
    )?;

    // Temel doğrulamalar
    if asset_a == asset_b {
        return Err(ApiError::from(nova_dex::DexError::InvalidTokenPair));
    }
    if initial_a == 0 || initial_b == 0 {
        return Err(ApiError::from(nova_dex::DexError::ZeroAmount));
    }

    // Oluşturma ücreti kontrolü
    let required = required_creation_fee_nnova(state.nova_price_cents)
        .map_err(ApiError::from)?;
    if creation_fee < required {
        return Err(ApiError::from(nova_dex::DexError::InsufficientCreationFee {
            required,
            provided: creation_fee,
        }));
    }

    // Pool ID türet ve benzersizlik kontrolü
    let pool_id = PoolId::from_assets(&asset_a, &asset_b);
    if db::pool_exists(&state.db, &pool_id).await? {
        return Err(ApiError::from(nova_dex::DexError::PoolAlreadyExists));
    }

    // LP token hesabı
    let initial_lp = amm::compute_lp_tokens_to_mint(initial_a, initial_b, 0, 0, 0)
        .map_err(ApiError::from)?;

    // Transaction: pool + LP pozisyonu
    let mut tx = state.db.begin().await.map_err(db::db_err)?;

    db::insert_pool(
        &mut tx, &pool_id, &asset_a, &asset_b, &creator,
        initial_a, initial_b, initial_lp, lock, now,
    ).await?;

    // Creator'ın ilk LP pozisyonu
    db::upsert_lp_position(
        &mut tx, &pool_id, &creator,
        initial_lp, initial_a, initial_b, now,
    ).await?;

    tx.commit().await.map_err(db::db_err)?;

    // Oluşturma ücreti treasury'ye (ayrı transaction)
    db::apply_fee_distribution(&state.db, &creator, 0, creation_fee).await?;

    tracing::info!("Yeni pool olusturuldu: {}", hex::encode(pool_id.0));
    Ok(Json(CreatePoolResponse { pool_id: hex::encode(pool_id.0) }))
}

pub async fn list_pools(
    State(state): State<AppState>,
) -> Json<Vec<PoolResponse>> {
    let rows = db::list_pools(&state.db).await.unwrap_or_default();
    let now  = now_secs();
    Json(rows.iter().map(|r| pool_row_to_dto(r, now)).collect())
}

pub async fn get_pool(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PoolResponse>, ApiError> {
    let pool_id = PoolId(parse_hex32(&id)?);
    let row = db::get_pool(&state.db, &pool_id)
        .await?
        .ok_or_else(|| ApiError::from(nova_dex::DexError::PoolNotFound))?;
    let now = now_secs();
    Ok(Json(pool_row_to_dto(&row, now)))
}

// ─── DTO Dönüşümü ────────────────────────────────────────────────────────────

pub fn pool_row_to_dto(row: &db::PoolRow, now: u64) -> PoolResponse {
    let duration = db_to_lock_duration(row.lock_duration_secs);
    PoolResponse {
        id:                   hex::encode(&row.id),
        asset_a:              hex::encode(&row.asset_a),
        asset_b:              hex::encode(&row.asset_b),
        creator:              hex::encode(&row.creator),
        reserve_a:            row.reserve_a.clone(),
        reserve_b:            row.reserve_b.clone(),
        total_lp:             row.total_lp.clone(),
        is_permanent:         row.lock_duration_secs == 0,
        lock_duration:        LockDurationDto::from(duration),
        lock_status:          lock_status_from_db(row.lock_duration_secs, row.lock_unlock_at, now),
        unlock_at:            db::db_to_unlock_at(row.lock_unlock_at),
        swap_count:           row.swap_count as u64,
        total_volume_a:       row.total_volume_a.clone(),
        total_fees_collected: row.total_fees_collected.clone(),
        created_at:           row.created_at as u64,
    }
}
