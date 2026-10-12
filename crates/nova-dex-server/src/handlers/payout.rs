use axum::{extract::State, http::HeaderMap, Json};

use crate::{
    db,
    dto::PayoutRecordResponse,
    error::{now_secs, ApiError},
    state::AppState,
};

pub async fn process_payouts(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<PayoutRecordResponse>>, ApiError> {
    // PAYOUT-001: Bearer token doğrulaması
    let secret = state.payout_secret.as_deref().ok_or(ApiError::Unauthorized)?;
    let provided = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if provided != Some(secret) {
        return Err(ApiError::Unauthorized);
    }

    let records = db::process_payouts(&state.db, now_secs()).await?;

    let mut response = Vec::with_capacity(records.len());
    for r in &records {
        let to_hex = hex::encode(r.creator.0);

        if let Some(result) = state.l1_client.send_nova(&to_hex, r.net_amount).await {
            match result {
                Ok(tx_hash) => tracing::info!("L1 transfer gonderildi: {to_hex} {tx_hash}"),
                Err(e)      => tracing::warn!("L1 transfer basarisiz ({to_hex}): {e}"),
            }
        }

        response.push(PayoutRecordResponse {
            creator:      to_hex,
            gross_amount: r.gross_amount.to_string(),
            net_amount:   r.net_amount.to_string(),
            transfer_fee: r.transfer_fee.to_string(),
            paid_at:      r.paid_at,
        });
    }

    Ok(Json(response))
}
