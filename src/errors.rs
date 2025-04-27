#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Logger error: {0}")]
    Logger(String),

    #[error("TLS error: {0}")]
    Tls(String),

    #[error("Proxy error: {0}")]
    Proxy(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] hyper::Error),

    #[error("Request error: {0}")]
    Request(#[from] hyper_util::client::legacy::Error),
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error::Proxy(s)
    }
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error::Proxy(s.to_string())
    }
} 