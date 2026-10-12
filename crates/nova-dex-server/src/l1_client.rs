//! L1 bakiye sorgulama istemcisi — nova-rpc JSON-RPC endpoint'ine HTTP istek atar.
//!
//! ## Kullanım
//!
//! DEX sunucu swap işleminden önce kullanıcının L1 bakiyesini kontrol edebilir.
//! `NOVA_L1_RPC_URL` env değişkeni ayarlı değilse L1 kontrolü **devre dışı** kalır
//! ve swap uyarı logu ile devam eder.
//!
//! ## Akış
//!
//! ```text
//! swap isteği → L1Client::get_balance(address) → nova-rpc:9944
//!                                                   ↓
//!                                             nova_getBalance
//!                                                   ↓
//!                                           hex bakiye string
//! ```

use std::time::Duration;

/// L1 RPC sunucu adresi. Ayarlı değilse L1 kontrolü atlanır.
const DEFAULT_L1_RPC_URL: &str = "http://localhost:9944";
const REQUEST_TIMEOUT_MS: u64 = 2_000;

/// Nova L1 JSON-RPC istemcisi.
#[derive(Clone)]
pub struct L1Client {
    inner: reqwest::Client,
    rpc_url: String,
    enabled: bool,
}

impl L1Client {
    /// `NOVA_L1_RPC_URL` env'den yapılandırılmış istemci oluştur.
    /// Env ayarlı değilse L1 kontrolü devre dışıdır (`enabled = false`).
    pub fn new() -> Self {
        let (rpc_url, enabled) = match std::env::var("NOVA_L1_RPC_URL") {
            Ok(url) => (url, true),
            Err(_) => (DEFAULT_L1_RPC_URL.to_string(), false),
        };

        let inner = reqwest::Client::builder()
            .timeout(Duration::from_millis(REQUEST_TIMEOUT_MS))
            .build()
            .expect("reqwest client oluşturulamadı");

        Self { inner, rpc_url, enabled }
    }

    /// L1 kontrolünün etkin olup olmadığını döndür.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// NOVA token transferi başlat (creator payout için).
    ///
    /// `NOVA_TREASURY_ADDRESS` env değişkeninden treasury adresini alır.
    /// - L1 **devre dışıysa** → `None` döner (transfer atlanır).
    /// - L1 **aktifse** → `Some(Ok(tx_hash))` veya `Some(Err(mesaj))` döner.
    pub async fn send_nova(&self, to_hex: &str, amount_nnova: u128) -> Option<Result<String, String>> {
        if !self.enabled {
            return None;
        }

        let from_hex = match std::env::var("NOVA_TREASURY_ADDRESS") {
            Ok(addr) => addr,
            Err(_) => {
                return Some(Err("NOVA_TREASURY_ADDRESS ayarli degil".to_string()));
            }
        };

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "nova_transfer",
            "params": [{
                "from":   from_hex,
                "to":     to_hex,
                "amount": format!("{amount_nnova:#x}"),
            }],
            "id": 1,
        });

        let result = async {
            let resp = self
                .inner
                .post(&self.rpc_url)
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("L1 transfer bağlantı hatası: {e}"))?;

            let json: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| format!("L1 transfer yanıt parse hatası: {e}"))?;

            if let Some(err) = json.get("error") {
                return Err(format!("L1 transfer hatası: {err}"));
            }

            json["result"]
                .as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| format!("L1 transfer geçersiz yanıt: {json}"))
        }
        .await;

        Some(result)
    }

    /// Verilen adresin L1 bakiyesini nNOVA cinsinden sorgula.
    ///
    /// - L1 **devre dışıysa** → `None` döner (çağıran check'i atlar).
    /// - L1 **aktifse** → `Some(Ok(balance))` veya `Some(Err(mesaj))` döner.
    pub async fn get_balance(&self, address_hex: &str) -> Option<Result<u128, String>> {
        if !self.enabled {
            return None; // L1 kontrolü atlanıyor — SERVER-004
        }

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "nova_getBalance",
            "params": [address_hex],
            "id": 1,
        });

        let result = async {
            let resp = self
                .inner
                .post(&self.rpc_url)
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("L1 RPC bağlantı hatası: {e}"))?;

            let json: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| format!("L1 RPC yanıt parse hatası: {e}"))?;

            // nova_getBalance hex string döner: "0x1a2b3c..."
            let hex = json["result"]
                .as_str()
                .ok_or_else(|| format!("L1 RPC geçersiz yanıt: {json}"))?;

            let trimmed = hex.trim_start_matches("0x");
            u128::from_str_radix(trimmed, 16)
                .map_err(|e| format!("L1 bakiye parse hatası ({hex}): {e}"))
        }
        .await;

        Some(result)
    }
}
