//! Ошибки крейта freecut-media.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("ошибка ввода/вывода: {0}")]
    Io(#[from] std::io::Error),

    #[error(
        "Не найдены ffmpeg и ffprobe. Добавьте папку FFmpeg в PATH или укажите FLOWCUT_FFMPEG_DIR."
    )]
    ToolchainMissing,

    #[error("ошибка обработки медиа: {0}")]
    Parse(String),

    #[error("неподдерживаемый файл: {path}")]
    Unsupported { path: String },

    #[error("{0}")]
    Message(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
