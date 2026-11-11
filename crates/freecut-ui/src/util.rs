//! Мелкие утилиты, используемые виджетами.

use egui::{Rect, Vec2};

/// UV-область текстуры, заполняющая `rect` без искажений (режим «cover»):
/// лишнее по краям обрезается, пропорции сохраняются.
#[must_use]
pub fn cover_uv(tex_size: Vec2, rect: Rect) -> Rect {
    let full = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    if tex_size.x <= 0.0 || tex_size.y <= 0.0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return full;
    }
    let tex_aspect = tex_size.x / tex_size.y;
    let rect_aspect = rect.width() / rect.height();
    if tex_aspect > rect_aspect {
        // Текстура шире: обрезаем края по X.
        let w = rect_aspect / tex_aspect;
        let x = (1.0 - w) * 0.5;
        Rect::from_min_max(egui::pos2(x, 0.0), egui::pos2(x + w, 1.0))
    } else {
        // Текстура уже/выше: обрезаем края по Y.
        let h = tex_aspect / rect_aspect;
        let y = (1.0 - h) * 0.5;
        Rect::from_min_max(egui::pos2(0.0, y), egui::pos2(1.0, y + h))
    }
}
