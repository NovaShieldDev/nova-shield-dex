//! Nova DEX — Uygulama Durumu (Stateless Server).
//!
//! ## Mimari
//!
//! Sprint 2 ile birlikte sunucu tamamen stateless hale geldi:
//! - **PostgreSQL**: Pool durumu, LP pozisyonları, treasury, creator fee
//! - **Redis**: MEV commit-reveal mempool'u, bot sayaçları
//! - **Bellek**: Yalnızca bağlantı havuzları ve sabit konfigürasyon
//!
//! Herhangi bir sayıda instance aynı PostgreSQL + Redis'e bağlanabilir.
//! Load balancer instance'lar arasında istekleri dağıtır.

use sqlx::PgPool;
use redis::aio::ConnectionManager;

use crate::l1_client::L1Client;
use crate::db;

/// Paylaşılan uygulama durumu — tüm handler'lar bu yapıyı klonlar.
///
/// `Clone` derive edilmiş: tüm alanlar Arc/pool/manager ile ucuz klonlanır.
#[derive(Clone)]
pub struct AppState {
    /// PostgreSQL bağlantı havuzu.
    pub db:              PgPool,
    /// Redis bağlantı yöneticisi (otomatik yeniden bağlanma).
    pub redis:           ConnectionManager,
    /// Nova L1 RPC istemcisi.
    pub l1_client:       L1Client,
    /// 1 NOVA'nın USD cent cinsinden fiyatı.
    pub nova_price_cents: u64,
    /// Payout Bearer token. None → 401 Unauthorized.
    pub payout_secret:   Option<String>,
    /// MEV slot hesabı için genesis zamanı (Unix milisaniye).
    pub genesis_time_ms: u64,
}

impl AppState {
    /// PostgreSQL ve Redis bağlantıları kurularak AppState oluştur.
    ///
    /// # Panics
    /// - DATABASE_URL geçersizse veya bağlantı başarısızsa
    /// - REDIS_URL geçersizse veya bağlantı başarısızsa
    /// - NOVA_PRICE_CENTS = 0 ise
    pub async fn new() -> Self {
        // ── PostgreSQL ────────────────────────────────────────────────────────
        let database_url = std::env::var("DATABASE_URL")
            .expect("DATABASE_URL ortam degiskeni ayarlanmamis");

        let db = PgPool::connect(&database_url)
            .await
            .unwrap_or_else(|e| {
                tracing::error!("PostgreSQL baglantisi kurulamadi: {e}");
                std::process::exit(1);
            });

        // Schema kur (idempotent: CREATE TABLE IF NOT EXISTS)
        db::setup_schema(&db).await.unwrap_or_else(|e| {
            tracing::error!("Veritabani schema kurulumu basarisiz: {e}");
            std::process::exit(1);
        });

        // ── Redis ─────────────────────────────────────────────────────────────
        let redis_url = std::env::var("REDIS_URL")
            .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

        let redis_client = redis::Client::open(redis_url.as_str())
            .unwrap_or_else(|e| {
                tracing::error!("Redis istemcisi olusturulamadi ({redis_url}): {e}");
                std::process::exit(1);
            });

        let redis = ConnectionManager::new(redis_client)
            .await
            .unwrap_or_else(|e| {
                tracing::error!("Redis baglantisi kurulamadi: {e}");
                std::process::exit(1);
            });

        // ── Genesis zamanı ────────────────────────────────────────────────────
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let genesis_secs = db::get_or_init_genesis_time(&db, now_secs)
            .await
            .unwrap_or_else(|e| {
                tracing::error!("Genesis zamani okunamadi: {e:?}");
                now_secs
            });

        let genesis_time_ms = genesis_secs * 1_000;

        // ── Diğer konfigürasyon ───────────────────────────────────────────────
        let nova_price_cents: u64 = std::env::var("NOVA_PRICE_CENTS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(250);

        if nova_price_cents == 0 {
            panic!("NOVA_PRICE_CENTS sifir olamaz");
        }

        let l1_client = L1Client::new();
        if l1_client.is_enabled() {
            tracing::info!("L1 bakiye kontrolu AKTIF");
        } else {
            tracing::warn!("L1 bakiye kontrolu DEVRE DISI — NOVA_L1_RPC_URL ayarli degil");
        }

        let payout_secret = std::env::var("PAYOUT_SECRET").ok();
        if payout_secret.is_none() {
            tracing::warn!("PAYOUT_SECRET ayarli degil — /v1/payouts/process devre disi (401)");
        }

        tracing::info!(
            "Nova DEX sunucu baslatildi | PostgreSQL: {} | Redis: {} | Genesis: {}s | NOVA: ${:.2}",
            database_url.split('@').next_back().unwrap_or("?"),
            redis_url,
            genesis_secs,
            nova_price_cents as f64 / 100.0,
        );

        Self {
            db,
            redis,
            l1_client,
            nova_price_cents,
            payout_secret,
            genesis_time_ms,
        }
    }
}
