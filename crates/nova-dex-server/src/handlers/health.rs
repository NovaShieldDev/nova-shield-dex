use axum::{extract::State, Json};

use crate::{db, dto::HealthResponse, state::AppState};

pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let pool_count = db::pool_count(&state.db).await.unwrap_or(0);
    Json(HealthResponse {
        status:     "ok",
        pool_count,
        version:    env!("CARGO_PKG_VERSION"),
    })
}
