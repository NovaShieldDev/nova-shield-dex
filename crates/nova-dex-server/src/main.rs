use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, post},
    Router,
};

/// Maksimum istek gövde boyutu (DoS koruması).
const MAX_BODY_BYTES: usize = 256 * 1024; // 256 KiB

use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber::EnvFilter;

mod auth;
mod db;
mod dto;
mod error;
mod handlers;
mod l1_client;
mod mempool_store;
mod state;

use state::AppState;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("nova_dex_server=debug,info")),
        )
        .init();

    // AppState: PostgreSQL + Redis bağlantıları kurulur
    let state = AppState::new().await;

    // SERVER-003: CORS kısıtlaması
    let cors = {
        let origin = std::env::var("ALLOWED_ORIGIN")
            .unwrap_or_else(|_| "http://localhost:3001".to_string());
        if origin == "*" {
            tracing::warn!("CORS: tum originlere izin veriliyor (gelistirme modu)");
            CorsLayer::permissive()
        } else {
            tracing::info!("CORS: izin verilen origin: {origin}");
            let header_val = origin
                .parse::<axum::http::HeaderValue>()
                .unwrap_or_else(|_| panic!("Gecersiz ALLOWED_ORIGIN: {origin}"));
            CorsLayer::new()
                .allow_origin(AllowOrigin::exact(header_val))
                .allow_methods([
                    axum::http::Method::GET,
                    axum::http::Method::POST,
                    axum::http::Method::DELETE,
                    axum::http::Method::OPTIONS,
                ])
                .allow_headers([
                    axum::http::header::CONTENT_TYPE,
                    axum::http::header::AUTHORIZATION,
                ])
        }
    };

    let app = Router::new()
        // Sağlık kontrolü
        .route("/health", get(handlers::health::health))
        // Pool işlemleri
        .route("/v1/pools",     post(handlers::pools::create_pool))
        .route("/v1/pools",     get(handlers::pools::list_pools))
        .route("/v1/pools/:id", get(handlers::pools::get_pool))
        // Likidite
        .route("/v1/pools/:id/liquidity",    post(handlers::liquidity::add_liquidity))
        .route("/v1/pools/:id/liquidity",    delete(handlers::liquidity::remove_liquidity))
        .route("/v1/pools/:id/lock/extend",  post(handlers::liquidity::extend_lock))
        .route("/v1/pools/:id/lp/:address",  get(handlers::liquidity::get_lp_position))
        // Creator ödemeleri
        .route("/v1/payouts/process", post(handlers::payout::process_payouts))
        // MEV koruması — commit-reveal (MEV-002)
        .route("/v1/mev/commit",          post(handlers::mev::commit))
        .route("/v1/mev/reveal",          post(handlers::mev::reveal))
        .route("/v1/mev/revealed/:slot_id", get(handlers::mev::get_revealed))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(cors)
        .with_state(state.clone());

    // ── Arka Plan: 24 Saatlik Creator Payout Görevi ──────────────────────────
    {
        let bg_state = state.clone();
        tokio::spawn(async move {
            const PAYOUT_INTERVAL_SECS: u64 = 86_400;
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(PAYOUT_INTERVAL_SECS)).await;
                tracing::info!("Otomatik creator payout dongusu baslatiliyor");

                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();

                match db::process_payouts(&bg_state.db, now).await {
                    Ok(records) => {
                        for r in &records {
                            let to_hex = hex::encode(r.creator.0);
                            if let Some(result) = bg_state.l1_client.send_nova(&to_hex, r.net_amount).await {
                                match result {
                                    Ok(tx)  => tracing::info!("Otomatik L1 transfer: {to_hex} {tx}"),
                                    Err(e)  => tracing::warn!("Otomatik L1 transfer basarisiz ({to_hex}): {e}"),
                                }
                            }
                        }
                        tracing::info!("Otomatik payout tamamlandi: {} kayit", records.len());
                    }
                    Err(e) => tracing::error!("Otomatik payout hatasi: {e:?}"),
                }
            }
        });
    }

    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr = format!("0.0.0.0:{port}");

    // TLS-001: TLS_CERT_PATH + TLS_KEY_PATH ayarlıysa HTTPS, yoksa HTTP
    let cert_path = std::env::var("TLS_CERT_PATH").ok();
    let key_path  = std::env::var("TLS_KEY_PATH").ok();

    match (cert_path, key_path) {
        (Some(cert), Some(key)) => {
            tracing::info!("TLS etkin: cert={cert}, key={key}");
            let tls_config = axum_server::tls_rustls::RustlsConfig::from_pem_file(&cert, &key)
                .await
                .unwrap_or_else(|e| {
                    tracing::error!("TLS sertifikasi yuklenemedi: {e}");
                    std::process::exit(1);
                });
            tracing::info!("Nova DEX API baslatiliyor: https://{addr}");
            axum_server::bind_rustls(addr.parse().unwrap(), tls_config)
                .serve(app.into_make_service())
                .await
                .unwrap();
        }
        _ => {
            tracing::info!("TLS devre disi — HTTP modunda calisiyor");
            tracing::info!("Nova DEX API baslatiliyor: http://{addr}");
            let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
            axum::serve(listener, app).await.unwrap();
        }
    }
}
