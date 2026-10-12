use axum::{
    extract::{Path, State},
    Json,
};
use nova_dex::amm;
use nova_dex::mev::SwapIntent;
use nova_dex::types::{Address, AssetId, FeeMode, PoolId};

use crate::{
    auth,
    db::{self, bytes32, str_to_u128},
    dto::{CommitRequest, CommitResponse, FeeModeDto, RevealRequest, SwapIntentResponse, SwapResponse},
    error::{now_ms, now_secs, parse_hex32, parse_u128, ApiError},
    mempool_store,
    state::AppState,
};

pub async fn commit(
    State(mut state): State<AppState>,
    Json(body): Json<CommitRequest>,
) -> Result<Json<CommitResponse>, ApiError> {
    let hash   = parse_hex32(&body.commitment_hash)?;
    let sender = Address(parse_hex32(&body.sender)?);

    let slot_id = mempool_store::submit_commit(
        &mut state.redis,
        hash,
        sender,
        state.genesis_time_ms,
        now_ms(),
    ).await?;

    Ok(Json(CommitResponse { slot_id }))
}

pub async fn reveal(
    State(mut state): State<AppState>,
    Json(body): Json<RevealRequest>,
) -> Result<Json<SwapResponse>, ApiError> {
    let sender    = Address(parse_hex32(&body.sender)?);
    let pool_id   = PoolId(parse_hex32(&body.pool_id)?);
    let asset_in  = AssetId(parse_hex32(&body.asset_in)?);
    let asset_out = AssetId(parse_hex32(&body.asset_out)?);
    let amount_in = parse_u128(&body.amount_in)?;
    let min_out   = parse_u128(&body.min_amount_out)?;
    let nonce     = parse_hex32(&body.nonce)?;

    // AUTH-001: ECDSA imza doğrulaması
    auth::verify_swap(
        &sender, &pool_id, &asset_in, &asset_out,
        &body.public_key, &body.signature,
        amount_in, min_out, &nonce,
        body.timestamp_secs, now_secs(),
    )?;

    let intent = SwapIntent {
        sender,
        pool_id,
        asset_in,
        asset_out,
        amount_in,
        min_amount_out: min_out,
        nonce,
    };

    // 1. Commit-reveal doğrulaması (Redis)
    mempool_store::reveal_intent(
        &mut state.redis,
        intent,
        state.genesis_time_ms,
        now_ms(),
    ).await?;

    // 2. L1 bakiye kontrolü (soft-fail, SERVER-001/004)
    if let Some(balance_result) = state.l1_client.get_balance(&body.sender).await {
        match balance_result {
            Ok(balance) if balance < amount_in => {
                return Err(ApiError::InvalidInput(format!(
                    "L1 bakiyesi yetersiz: {balance} < {amount_in} nNOVA"
                )));
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("L1 bakiye sorgusu basarisiz (swap devam ediyor): {e}"),
        }
    }

    // 3. Bot sayacı (Redis) — fee modunu belirle
    let bot_count = mempool_store::incr_bot_counter(&mut state.redis, &sender, now_secs())
        .await
        .unwrap_or(1); // Redis hatası → normal fee
    let fee_mode  = FeeMode::from_daily_count(bot_count as u64);

    // 4. Swap — pool satır kilidi ile (PostgreSQL transaction)
    let mut tx = state.db.begin().await.map_err(db::db_err)?;

    let pool_row = db::get_pool_for_update(&mut tx, &pool_id)
        .await?
        .ok_or_else(|| ApiError::from(nova_dex::DexError::PoolNotFound))?;

    let asset_a_db = AssetId(bytes32(&pool_row.asset_a));
    let zero_for_one = asset_a_db == asset_in;

    let reserve_a = str_to_u128(&pool_row.reserve_a);
    let reserve_b = str_to_u128(&pool_row.reserve_b);

    let (reserve_in, reserve_out) = if zero_for_one {
        (reserve_a, reserve_b)
    } else {
        (reserve_b, reserve_a)
    };

    // 5. AMM hesabı (slippage dahil)
    let comp = amm::compute_swap(reserve_in, reserve_out, amount_in, fee_mode)
        .map_err(ApiError::from)?;

    if min_out > 0 && comp.amount_out < min_out {
        return Err(ApiError::from(nova_dex::DexError::SlippageExceeded {
            min_amount_out:    min_out,
            actual_amount_out: comp.amount_out,
        }));
    }

    // 6. Rezervleri güncelle
    let (new_reserve_a, new_reserve_b, new_volume_a) = if zero_for_one {
        (
            reserve_a.saturating_add(comp.amount_in_net),
            reserve_b.saturating_sub(comp.amount_out),
            str_to_u128(&pool_row.total_volume_a).saturating_add(amount_in),
        )
    } else {
        (
            reserve_a.saturating_sub(comp.amount_out),
            reserve_b.saturating_add(comp.amount_in_net),
            str_to_u128(&pool_row.total_volume_a).saturating_add(comp.amount_out),
        )
    };

    let new_fees = str_to_u128(&pool_row.total_fees_collected).saturating_add(comp.fee_amount);

    db::update_pool_after_swap(
        &mut tx, &pool_id,
        new_reserve_a, new_reserve_b,
        new_fees, new_volume_a,
        (pool_row.swap_count as u64).saturating_add(1),
    ).await?;

    tx.commit().await.map_err(db::db_err)?;

    // 7. Fee dağıtımı (ayrı transaction, pool güncelleme sonrası)
    let creator = Address(bytes32(&pool_row.creator));
    db::apply_fee_distribution(
        &state.db,
        &creator,
        comp.creator_fee(),
        comp.treasury_fee(),
    ).await?;

    Ok(Json(SwapResponse {
        amount_out:       comp.amount_out.to_string(),
        amount_in_net:    comp.amount_in_net.to_string(),
        fee_amount:       comp.fee_amount.to_string(),
        creator_fee:      comp.creator_fee().to_string(),
        treasury_fee:     comp.treasury_fee().to_string(),
        fee_mode:         FeeModeDto::from(comp.fee_mode),
        price_impact_bps: comp.price_impact_bps,
    }))
}

pub async fn get_revealed(
    State(mut state): State<AppState>,
    Path(slot_id): Path<u64>,
) -> Json<Vec<SwapIntentResponse>> {
    let intents = mempool_store::get_revealed_for_slot(&mut state.redis, slot_id).await;

    let response = intents
        .into_iter()
        .map(|i| SwapIntentResponse {
            sender:         hex::encode(i.sender.0),
            pool_id:        hex::encode(i.pool_id.0),
            asset_in:       hex::encode(i.asset_in.0),
            asset_out:      hex::encode(i.asset_out.0),
            amount_in:      i.amount_in.to_string(),
            min_amount_out: i.min_amount_out.to_string(),
            nonce:          hex::encode(i.nonce),
        })
        .collect();

    Json(response)
}
