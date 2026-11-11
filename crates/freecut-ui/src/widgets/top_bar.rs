//! Верхняя панель: имя проекта, undo/redo, недавние проекты, экспорт.

use egui::{Align, Align2, CornerRadius, Layout, Sense, Ui, Vec2};

use crate::{
    app::UiState,
    icons::{self, Icon},
    project_ui::{ProjectAction, reveal},
    theme::Palette,
};

/// Высота верхней панели.
pub const BAR_HEIGHT: f32 = 46.0;

pub fn show(ui: &mut Ui, st: &mut UiState) {
    egui::Panel::top("top_bar")
        .exact_size(BAR_HEIGHT)
        .frame(
            egui::Frame::new()
                .fill(Palette::PANEL)
                // Поля у краёв окна: контент не прилипает к рамке.
                .inner_margin(egui::Margin {
                    left: 14,
                    right: 14,
                    top: 0,
                    bottom: 0,
                }),
        )
        .show_separator_line(false)
        .show(ui, |ui| {
            ui.columns(3, |columns| {
                // Колонка 1: имя проекта.
                columns[0].with_layout(Layout::left_to_right(Align::Center), |ui| {
                    project_title(ui, st);
                });

                // Колонка 2: undo/redo по центру (ручное смещение — надёжно).
                columns[1].with_layout(Layout::left_to_right(Align::Center), |ui| {
                    // 2 кнопки по 30 + 8px item_spacing + 8px ручного интервала.
                    let pair_w = 30.0 + 8.0 + 8.0 + 30.0;
                    ui.add_space(((ui.available_width() - pair_w) * 0.5).max(0.0));
                    let can_undo = st.can_undo();
                    let can_redo = st.can_redo();
                    let undo = icons::icon_button(ui, Icon::Undo, Vec2::new(30.0, 28.0), can_undo);
                    if undo.clicked() {
                        st.undo();
                    }
                    ui.add_space(8.0);
                    let redo = icons::icon_button(ui, Icon::Redo, Vec2::new(30.0, 28.0), can_redo);
                    if redo.clicked() {
                        st.redo();
                    }
                });

                // Колонка 3: справа — недавние и экспорт.
                columns[2].with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // RTL-поток: первый виджет встаёт у правого края сам.
                    let export = accent_button(ui, Vec2::new(104.0, 30.0), "Экспорт");
                    if export.clicked() {
                        st.project.export_open = !st.project.export_open;
                    }
                    ui.menu_button("Проект", |ui| {
                        for (label, action) in [
                            ("Новый   Ctrl+N", ProjectAction::New),
                            ("Открыть…   Ctrl+O", ProjectAction::Open),
                            ("Сохранить   Ctrl+S", ProjectAction::Save),
                        ] {
                            if ui.button(label).clicked() {
                                st.project.action = Some(action);
                                ui.close();
                            }
                        }
                        if !st.project.settings.recents.is_empty() {
                            ui.separator();
                            ui.label("Недавние");
                            for path in &st.project.settings.recents {
                                let label = path
                                    .file_name()
                                    .unwrap_or(path.as_os_str())
                                    .to_string_lossy();
                                if ui
                                    .button(label)
                                    .on_hover_text(path.display().to_string())
                                    .clicked()
                                {
                                    st.project.action =
                                        Some(ProjectAction::OpenRecent(path.clone()));
                                    ui.close();
                                }
                            }
                        }
                    });
                });
            });
        });
}

/// Акцентная кнопка (Экспорт): своя отрисовка, чтобы ховер/нажатие были заметны.
/// Участвует в обычном потоке Ui (в т.ч. RTL), размер задан явно.
fn accent_button(ui: &mut Ui, size: Vec2, label: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let bg = if resp.is_pointer_button_down_on() {
        Palette::ACCENT_PRESSED
    } else if resp.hovered() {
        Palette::ACCENT_HOVER
    } else {
        Palette::ACCENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(7), bg);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        egui::TextStyle::Button.resolve(ui.style()),
        Palette::ACCENT_TEXT,
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

fn project_title(ui: &mut Ui, st: &mut UiState) {
    ui.add_space(2.0);

    if st.renaming {
        let response = ui.add(
            egui::TextEdit::singleline(&mut st.doc.name)
                .desired_width(220.0)
                .font(egui::TextStyle::Button),
        );
        if response.lost_focus() {
            st.renaming = false;
            st.dirty = true;
        }
    } else {
        let title = st.doc.display_name();
        let dirty_mark = if st.dirty { " •" } else { "" };
        let label = format!("{title}{dirty_mark}");
        let galley = ui.painter().layout_no_wrap(
            label,
            egui::TextStyle::Button.resolve(ui.style()),
            Palette::TEXT,
        );
        let width = (galley.size().x + 16.0).min(ui.available_width());
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click());
        if resp.hovered() && !st.renaming {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(6), Palette::SURFACE);
        }
        ui.painter().with_clip_rect(rect).galley(
            egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y * 0.5),
            galley,
            Palette::TEXT,
        );
        if resp.clicked() {
            st.renaming = true;
        }
        resp.on_hover_cursor(egui::CursorIcon::Text)
            .on_hover_text("Нажмите, чтобы переименовать проект");
    }
}

pub fn status(ui: &mut Ui, st: &mut UiState) {
    let missing = st
        .doc
        .assets
        .iter()
        .filter(|asset| !asset.path.is_file())
        .count();
    if st.notice.is_none() && missing == 0 {
        return;
    }
    egui::Panel::top("editor_status")
        .frame(
            egui::Frame::new()
                .fill(Palette::PANEL_UPPER)
                .inner_margin(8),
        )
        .show(ui, |ui| {
            if missing != 0 {
                ui.menu_button(
                    format!("Отсутствуют исходники ({missing})"),
                    |ui| {
                        for asset in st.doc.assets.iter().filter(|asset| !asset.path.is_file()) {
                            if ui
                                .button(&asset.display_name)
                                .on_hover_text(asset.path.display().to_string())
                                .clicked()
                            {
                                st.project.action = Some(ProjectAction::Relink(asset.id));
                                ui.close();
                            }
                        }
                    },
                );
            }
            if let Some(notice) = st.notice.clone() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(notice);
                    if let Some(path) = &st.project.completed
                        && ui.button("Показать в папке").clicked()
                        && let Err(error) = reveal(path)
                    {
                        st.notice = Some(error.to_string());
                    }
                    if ui.small_button("Закрыть").clicked() {
                        st.notice = None;
                        st.project.completed = None;
                    }
                });
            }
        });
}
