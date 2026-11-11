//! Панели приложения.

pub mod inspector;
pub mod library;
pub mod preview;
pub mod timeline;
pub mod top_bar;

use egui::{CornerRadius, Ui};

use crate::theme::Palette;

/// Панель-«карточка» с фоном и скруглением.
pub fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(Palette::PANEL_UPPER)
        .stroke(egui::Stroke::new(1.0, Palette::STROKE_SOFT))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(12))
        .outer_margin(egui::Margin {
            left: 0,
            right: 0,
            top: 0,
            bottom: 8,
        })
        .show(ui, add);
}
