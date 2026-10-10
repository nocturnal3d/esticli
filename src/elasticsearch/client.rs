use crate::error::{EstiCliError, Result};
use crate::models::{ClusterHealth, IndexDetails, NodeStats};
use url::Url;

use super::stats::Snapshot;

#[derive(Clone)]
pub enum AuthConfig {
    None,
    Basic { username: String, password: String },
    ApiKey(String),
}

/// A connection to one Elasticsearch cluster.
///
/// Holds no per-poll state, so it is cheap to clone (`reqwest::Client` is
/// reference-counted internally) and every clone can issue requests at the
/// same time. The polling task and the details popup each use their own
/// clone; the rate baseline that polling needs lives in
/// `stats::RateTracker` instead, so a slow `_stats` request never holds up an
/// unrelated details fetch.
#[derive(Clone)]
pub struct EsClient {
    pub(crate) client: reqwest::Client,
    pub(crate) base_url: Url,
    pub(crate) auth: AuthConfig,
}

/// Everything one refresh tick brings back, each part independently: a
/// failing `_cluster/health` or `_nodes/stats` must not throw away a `_stats`
/// reading that arrived fine, since the table, header rate and graph only
/// depend on that one.
pub struct Tick {
    pub snapshot: Result<Snapshot>,
    pub health: Result<ClusterHealth>,
    /// `None` when `_nodes/stats` wasn't requested (its panel isn't on
    /// screen), as opposed to `Some(Ok(vec![]))`, which would mean the
    /// cluster genuinely reported no nodes.
    pub nodes: Option<Result<Vec<NodeStats>>>,
}

impl EsClient {
    pub fn new(
        base_url: &str,
        auth: AuthConfig,
        insecure: bool,
        ca_cert: Option<std::path::PathBuf>,
    ) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .danger_accept_invalid_certs(insecure)
            .gzip(true)
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30));

        if let Some(ca_path) = ca_cert {
            let ca_data = std::fs::read(&ca_path).map_err(|e| {
                EstiCliError::Config(format!(
                    "Failed to read CA certificate {}: {}",
                    ca_path.display(),
                    e
                ))
            })?;
            // `from_pem_bundle` rejects a file with no certificate in it,
            // where `from_pem` would accept it and only fail at the first
            // handshake with an unrelated-looking TLS error.
            let certs = reqwest::Certificate::from_pem_bundle(&ca_data).map_err(|e| {
                EstiCliError::Config(format!(
                    "Failed to parse CA certificate {}: {}",
                    ca_path.display(),
                    e
                ))
            })?;
            if certs.is_empty() {
                return Err(EstiCliError::Config(format!(
                    "No PEM certificate found in {}",
                    ca_path.display()
                )));
            }
            for cert in certs {
                builder = builder.add_root_certificate(cert);
            }
        }

        Ok(Self {
            client: builder.build()?,
            base_url: parse_base_url(base_url)?,
            auth,
        })
    }

    /// The cluster URL as it is safe to put on screen: any credentials
    /// embedded in it (`https://user:pass@host`) are dropped, since a
    /// `top`-like dashboard ends up screen-shared and screenshotted.
    pub fn display_url(&self) -> String {
        let mut url = self.base_url.clone();
        // Neither call can fail on an http(s) URL, which `parse_base_url`
        // guarantees this is.
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.as_str().trim_end_matches('/').to_string()
    }

    /// Builds a request URL by appending `segments` to the base URL's path.
    ///
    /// Each segment is percent-encoded, so an index name containing `%`,
    /// `&`, `#` or non-ASCII characters reaches Elasticsearch as one literal
    /// path segment, and a base URL with a path prefix (an Elasticsearch
    /// behind a gateway at `https://gw/es`) keeps that prefix. Joining a
    /// relative path onto the base instead would replace its last segment.
    pub(crate) fn endpoint(&self, segments: &[&str]) -> Url {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .expect("parse_base_url only accepts URLs that can be a base")
            .pop_if_empty()
            .extend(segments);
        url
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
            return Err(EstiCliError::Api {
                status,
                body: summarize_error_body(&body),
            });
        }

        let body = response.bytes().await?;
        serde_json::from_slice(&body).map_err(EstiCliError::from)
    }

    /// Fetches everything one refresh tick needs, issuing the requests
    /// concurrently rather than one after the other — they are independent,
    /// and on a slow cluster serialising them multiplied the latency of every
    /// tick.
    ///
    /// `want_nodes` gates the `_nodes/stats` request on the nodes panel
    /// actually being on screen; when it is `false` no request is made.
    pub async fn fetch_tick(&self, want_nodes: bool) -> Tick {
        let (snapshot, health, nodes) = tokio::join!(
            super::stats::fetch_snapshot(self),
            super::stats::fetch_cluster_health(self),
            async {
                if want_nodes {
                    Some(super::nodes::fetch_node_stats(self).await)
                } else {
                    None
                }
            },
        );

        Tick {
            snapshot,
            health,
            nodes,
        }
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
}

/// Parses `-u`, accepting only URLs `endpoint` can append paths to.
///
/// A missing scheme is the common mistake (`-u localhost:9200`), and the URL
/// parser happily reads that as the scheme `localhost:` with no host, which
/// would otherwise only surface as a baffling request error later.
fn parse_base_url(raw: &str) -> Result<Url> {
    let url = Url::parse(raw)?;
    match url.scheme() {
        "http" | "https" if url.host().is_some() => Ok(url),
        "http" | "https" => Err(EstiCliError::Config(format!(
            "Invalid Elasticsearch URL {:?}: no host",
            raw
        ))),
        _ => Err(EstiCliError::Config(format!(
            "Invalid Elasticsearch URL {:?}: expected http:// or https:// (e.g. http://{})",
            raw, raw
        ))),
    }
}

/// Shortens an error response body for the one-line header.
///
/// Elasticsearch's own errors are JSON with a `reason` that says everything
/// worth saying; anything else (an HTML page from a proxy or load balancer)
/// is cut down rather than dumped whole into the UI.
fn summarize_error_body(body: &str) -> String {
    const MAX_LEN: usize = 300;

    let reason = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|json| {
            let error = json.get("error")?;
            error
                .get("reason")
                .and_then(|reason| reason.as_str())
                .or_else(|| error.as_str())
                .map(str::to_string)
        });

    let text = reason.unwrap_or_else(|| body.split_whitespace().collect::<Vec<_>>().join(" "));
    if text.chars().count() > MAX_LEN {
        let truncated: String = text.chars().take(MAX_LEN).collect();
        format!("{}…", truncated)
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(url: &str) -> EsClient {
        EsClient::new(url, AuthConfig::None, false, None).unwrap()
    }

    #[test]
    fn test_endpoint_keeps_a_path_prefix() {
        for base in ["https://gw.example.com/es", "https://gw.example.com/es/"] {
            assert_eq!(
                client(base).endpoint(&["_stats", "docs"]).as_str(),
                "https://gw.example.com/es/_stats/docs",
                "base {base}"
            );
        }
        assert_eq!(
            client("http://localhost:9200")
                .endpoint(&["_cluster", "health"])
                .as_str(),
            "http://localhost:9200/_cluster/health"
        );
    }

    #[test]
    fn test_endpoint_encodes_index_names_as_one_segment() {
        let url = client("http://localhost:9200").endpoint(&["100%&more#x/y", "_settings"]);
        assert_eq!(url.path(), "/100%25&more%23x%2Fy/_settings");
        assert!(url.fragment().is_none());
    }

    #[test]
    fn test_base_url_must_be_http() {
        assert!(EsClient::new("localhost:9200", AuthConfig::None, false, None).is_err());
        assert!(EsClient::new("ftp://host", AuthConfig::None, false, None).is_err());
        assert!(EsClient::new("https://host:9243", AuthConfig::None, false, None).is_ok());
    }

    #[test]
    fn test_display_url_hides_credentials() {
        assert_eq!(
            client("https://elastic:secret@es.example.com:9243/").display_url(),
            "https://es.example.com:9243"
        );
        assert_eq!(
            client("http://localhost:9200").display_url(),
            "http://localhost:9200"
        );
    }

    #[test]
    fn test_error_body_prefers_the_es_reason() {
        let body = r#"{"error":{"root_cause":[],"type":"security_exception","reason":"action [cluster:monitor/health] is unauthorized"},"status":403}"#;
        assert_eq!(
            summarize_error_body(body),
            "action [cluster:monitor/health] is unauthorized"
        );

        let html = format!("<html>\n<body>{}</body></html>", "x".repeat(1000));
        let summary = summarize_error_body(&html);
        assert!(summary.chars().count() <= 301);
        assert!(!summary.contains('\n'));
    }
}
