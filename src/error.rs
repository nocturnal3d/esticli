use thiserror::Error;

/// Custom error types for the EstiCLI application.
#[derive(Error, Debug)]
pub enum EstiCliError {
    /// Error indicating a failure to connect to Elasticsearch.
    ///
    /// reqwest's own message stops at "error sending request for url (…)",
    /// with the full request URL in it; the part that says what actually went
    /// wrong (connection refused, DNS, an untrusted certificate, a timeout)
    /// is further down its source chain. That cause is what is shown — on a
    /// one-line header the URL would push it off screen, and the header
    /// already names the cluster.
    #[error("cannot reach Elasticsearch: {}", cause(.0))]
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

    /// A problem with the command-line configuration (URL, CA file), worded
    /// for the user to act on.
    #[error("{0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, EstiCliError>;

/// Joins every error beneath `error` into one line, skipping a level that
/// only repeats the one above, or falls back to `error` itself when it has
/// no source.
fn cause(error: &dyn std::error::Error) -> String {
    let mut messages: Vec<String> = Vec::new();
    let mut source = error.source();
    while let Some(cause) = source {
        let message = cause.to_string();
        if messages.last().is_none_or(|last| !last.contains(&message)) {
            messages.push(message);
        }
        source = cause.source();
    }
    if messages.is_empty() {
        error.to_string()
    } else {
        messages.join(": ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Layer(&'static str, Option<Box<Layer>>);

    impl std::fmt::Display for Layer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl std::error::Error for Layer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.1.as_deref().map(|e| e as _)
        }
    }

    #[test]
    fn test_cause_reaches_the_root_cause() {
        let error = Layer(
            "error sending request for url (http://a-very-long-url)",
            Some(Box::new(Layer(
                "client error (Connect)",
                Some(Box::new(Layer("Connection refused (os error 61)", None))),
            ))),
        );
        assert_eq!(
            cause(&error),
            "client error (Connect): Connection refused (os error 61)"
        );
        assert_eq!(cause(&Layer("alone", None)), "alone");
    }
}
