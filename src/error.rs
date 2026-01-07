use thiserror::Error;

/// Custom error types for the EstiCLI application.
#[derive(Error, Debug)]
pub enum EstiCliError {
    /// Error indicating a failure to connect to Elasticsearch.
    #[error("Elasticsearch connection failed: {0}")]
    Connection(#[from] reqwest::Error),

    /// Error indicating a non-2xx API response from Elasticsearch.
    #[error("API error (Status {status}): {body}")]
    Api {
        status: reqwest::StatusCode,
        body: String,
    },

    #[error("Failed to parse Elasticsearch response: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("URL parsing error: {0}")]
    Url(#[from] url::ParseError),

    #[error("Internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, EstiCliError>;
