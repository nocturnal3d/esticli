use crate::error::{EstiCliError, Result};
use crate::models::{ClusterHealth, IndexDetails, IndexRate, IndexSnapshot};
use url::Url;

#[derive(Clone)]
pub enum AuthConfig {
    None,
    Basic { username: String, password: String },
    ApiKey(String),
}

pub struct EsClient {
    pub(crate) client: reqwest::Client,
    pub(crate) base_url: Url,
    pub(crate) auth: AuthConfig,
    pub(crate) previous_snapshot: Option<(
        std::time::Instant,
        std::collections::HashMap<String, IndexSnapshot>,
    )>,
}

impl EsClient {
    pub fn new(
        base_url: String,
        auth: AuthConfig,
        insecure: bool,
        ca_cert: Option<std::path::PathBuf>,
    ) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .danger_accept_invalid_certs(insecure)
            .gzip(true)
            .timeout(std::time::Duration::from_secs(30));

        if let Some(ca_path) = ca_cert {
            let ca_data = std::fs::read(&ca_path).map_err(|e| {
                EstiCliError::Internal(format!("Failed to read CA certificate: {}", e))
            })?;
            let cert = reqwest::Certificate::from_pem(&ca_data).map_err(|e| {
                EstiCliError::Internal(format!("Failed to parse CA certificate: {}", e))
            })?;
            builder = builder.add_root_certificate(cert);
        }

        let client = builder.build()?;
        let url = Url::parse(base_url.trim_end_matches('/'))?;

        Ok(Self {
            client,
            base_url: url,
            auth,
            previous_snapshot: None,
        })
    }

    pub(crate) fn auth_request(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.auth {
            AuthConfig::None => request,
            AuthConfig::Basic { username, password } => {
                request.basic_auth(username, Some(password))
            }
            AuthConfig::ApiKey(key) => request.header("Authorization", format!("ApiKey {}", key)),
        }
    }

    // Helper to send a request and parse the response as JSON with error handling
    pub(crate) async fn send_json<T>(&self, request: reqwest::RequestBuilder) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let response = self.auth_request(request).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(EstiCliError::Api { status, body });
        }

        let body = response.bytes().await?;
        serde_json::from_slice(&body).map_err(EstiCliError::from)
    }

    pub async fn fetch_index_rates(&mut self) -> Result<Vec<IndexRate>> {
        super::stats::fetch_index_rates(self).await
    }

    pub async fn fetch_index_details(
        &self,
        index_name: &str,
        doc_count: u64,
        rate_per_sec: f64,
        size_bytes: u64,
    ) -> Result<IndexDetails> {
        super::details::fetch_index_details(self, index_name, doc_count, rate_per_sec, size_bytes)
            .await
    }

    pub async fn fetch_cluster_health(&mut self) -> Result<ClusterHealth> {
        super::stats::fetch_cluster_health(self).await
    }
}
