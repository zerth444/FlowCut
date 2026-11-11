//! Ошибки крейта freecut-audio.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("аудиоустройство недоступно: {0}")]
    DeviceUnavailable(String),

    #[error("{0}")]
    Message(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    /// Из ошибки инициализации аудиоустройства.
    #[must_use]
    pub fn from_device(err: impl std::fmt::Display) -> Self {
        Self::DeviceUnavailable(err.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::Message(err.to_string())
    }
}
