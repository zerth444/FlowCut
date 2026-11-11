//! FreeCut media: зондирование источников (ffprobe), декодирование превью,
//! генерация миниатюр и waveform, экспорт через ffmpeg.
//!
//! Связь с медиа-инструментами — через subprocess ffmpeg/ffprobe: это
//! изолирует падения декодеров от приложения и не привязывает сборку
//! к FFI-версиям libav.

pub mod artwork;
pub mod decoder;
pub mod error;
pub mod export;
pub mod probe;
pub mod snapshot;

pub fn color_filter(filters: freecut_core::Filters) -> String {
    format!(
        "eq=brightness={}:contrast={}:saturation={}",
        filters.brightness, filters.contrast, filters.saturation
    )
}

pub use error::{Error, Result};
