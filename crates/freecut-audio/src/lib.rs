//! FreeCut audio: микширование треков и воспроизведение единой звуковой
//! дорожки в превью (cpal).

pub mod error;
pub mod player;

pub use error::{Error, Result};
pub use player::{AudioPlayer, SpawnSource};
