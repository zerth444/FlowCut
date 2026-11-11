//! FreeCut core: проект, таймлайн, эффекты, команды undo/redo.
//!
//! Крейт не зависит от GUI. Всё, что связано с моделью данных,
//! живёт здесь, чтобы её можно было покрыть unit-тестами и переиспользовать
//! в утилитах (CLI-экспорт и т.п.) без подтягивания egui.

pub mod error;
pub mod model;
pub mod project;

pub use error::{Error, Result};
pub use model::*;
