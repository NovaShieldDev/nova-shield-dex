use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use nova_dex::DexError;
use serde_json::json;

#[derive(Debug)]
pub enum ApiError {
    Dex(DexError),
    InvalidInput(String),
    Unauthorized,
}

impl From<DexError> for ApiError {
    fn from(e: DexError) -> Self {
        Self::Dex(e)
    }
}

impl ApiError {
    pub fn invalid_input(msg: impl Into<String>) -> Self {
        Self::InvalidInput(msg.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            ApiError::Dex(e) => {
                let status = match e {
                    DexError::PoolNotFound
                    | DexError::LiquidityProviderNotFound
                    | DexError::CommitNotFound => StatusCode::NOT_FOUND,
                    DexError::Unauthorized => StatusCode::FORBIDDEN,
                    DexError::PoolAlreadyExists
                    | DexError::CommitAlreadyExists => StatusCode::CONFLICT,
                    DexError::MempoolFull => StatusCode::SERVICE_UNAVAILABLE,
                    _ => StatusCode::BAD_REQUEST,
                };
                (status, e.to_string())
            }
            ApiError::InvalidInput(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "Yetkisiz: geçersiz veya eksik token".to_string()),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

// ─── Parse ve Zaman Yardimcilari ─────────────────────────────────────────────

pub fn parse_hex32(s: &str) -> Result<[u8; 32], ApiError> {
    let bytes = hex::decode(s)
        .map_err(|_| ApiError::invalid_input(format!("gecersiz hex: {s}")))?;
    bytes
        .try_into()
        .map_err(|_| ApiError::invalid_input(format!("32 byte olmali: {s}")))
}

pub fn parse_u128(s: &str) -> Result<u128, ApiError> {
    s.parse::<u128>()
        .map_err(|_| ApiError::invalid_input(format!("gecersiz sayi: {s}")))
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
