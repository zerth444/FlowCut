//! Единая ошибка крейта.

use crate::Id;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("ошибка ввода/вывода: {0}")]
    Io(#[from] std::io::Error),

    #[error("ошибка сериализации: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("ассет {id} отсутствует в проекте")]
    MissingAsset { id: Id },

    #[error("файл источника не найден: {path}")]
    SourceMissing { path: String },

    #[error("неподдерживаемый файл: {path}")]
    Unsupported { path: String },

    #[error("{0}")]
    Message(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
