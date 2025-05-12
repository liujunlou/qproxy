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

    #[error("Service error: {0}")]
    ServiceError(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] hyper::Error),

    #[error("Request error: {0}")]
    Request(#[from] reqwest::Error),

    #[error("Request error: {0}")]
    Request2(#[from] hyper_util::client::legacy::Error),

    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Service discovery error: {0}")]
    ServiceDiscovery(String),

    #[error("SystemTime error: {0}")]
    SystemTime(#[from] std::time::SystemTimeError),

    /// MQTT 错误
    #[error("MQTT error: {0}")]
    Mqtt(String),
    #[error("Decode error: {0}")]
    MqttDecodeError(String),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as StdError;
    use std::io;

    #[test]
    fn test_error_display() {
        let err = Error::ServiceDiscovery("test error".to_string());
        assert_eq!(err.to_string(), "Service discovery error: test error");

        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let err = Error::Io(io_err);
        assert!(err.to_string().contains("I/O error"));
    }

    #[test]
    fn test_error_source() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let err = Error::Io(io_err);
        assert!(err.source().is_some());

        let err = Error::ServiceDiscovery("test error".to_string());
        assert!(err.source().is_none());
    }

    #[test]
    fn test_error_conversion() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::Io(_)));
    }
} 