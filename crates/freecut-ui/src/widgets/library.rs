//! Левая панель: медиатека (вкладки Медиа/Аудио/Эффекты), импорт, drag&drop.

use egui::{Align2, Color32, CornerRadius, DragAndDrop, Sense, Stroke, Ui, Vec2};

use freecut_core::{Asset, AssetKind};

use crate::{
    app::{AssetDragPayload, LibraryTab as Tab, UiState},
    icons,
    theme::Palette,
};

/// Ширина левой панели.
pub const PANEL_WIDTH: f32 = 262.0;

pub fn show(ui: &mut Ui, st: &mut UiState) {
    egui::Panel::left("library_panel")
        .default_size(PANEL_WIDTH)
        .min_size(160.0)
        .max_size(480.0)
        .frame(
            egui::Frame::new()
                .fill(Palette::PANEL)
                .inner_margin(egui::Margin::same(12)),
        )
        .show_separator_line(false)
        .show(ui, |ui| {
            ui.vertical(|ui| {
                widgets_segmented(ui, st);
                ui.add_space(6.0);
                if st.importing {
                    ui.label("Импорт файлов…");
                }

                match st.library_tab {
                    Tab::Media => media_tab(ui, st, false),
                    Tab::Audio => media_tab(ui, st, true),
                    Tab::Effects => {
                        ui.label(crate::theme::section_title("Затемнение проекта"));
                        let has_video = st
                            .doc
                            .timeline
                            .video_tracks()
                            .any(|track| !track.clips.is_empty());
                        if ui
                            .add_enabled(has_video, egui::Button::new("Затемнение в начале"))
                            .clicked()
                        {
                            st.project_fade(false);
                        }
                        if ui
                            .add_enabled(has_video, egui::Button::new("Затемнение в конце"))
                            .clicked()
                        {
                            st.project_fade(true);
                        }
                        ui.add_space(6.0);
                        ui.label("Цвет, затухание и переходы клипа доступны в инспекторе.");
                    }
                }
            });
        });
}

fn widgets_segmented(ui: &mut Ui, st: &mut UiState) {
    let items = ["Медиа", "Аудио", "Эффекты"];
    let selected = match st.library_tab {
        Tab::Media => 0,
        Tab::Audio => 1,
        Tab::Effects => 2,
    };

    ui.horizontal(|ui| {
        for (index, label) in items.iter().enumerate() {
            let is_selected = index == selected;
            let width = ((ui.available_width() - 2.0 * ui.spacing().item_spacing.x)
                / (items.len() - index) as f32)
                .clamp(38.0, 100.0);
            let height = 30.0;
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
            let bg = if is_selected {
                Palette::SURFACE_HOVER
            } else if resp.hovered() {
                Palette::SURFACE
            } else {
                Color32::TRANSPARENT
            };
            if bg != Color32::TRANSPARENT {
                ui.painter().rect_filled(rect, CornerRadius::same(7), bg);
            }
            let color = if is_selected || resp.hovered() {
                Palette::TEXT
            } else {
                Palette::TEXT_DIM
            };
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                *label,
                egui::TextStyle::Button.resolve(ui.style()),
                color,
            );
            if resp.clicked() {
                st.library_tab = match index {
                    1 => Tab::Audio,
                    2 => Tab::Effects,
                    _ => Tab::Media,
                };
            }
        }
    });
}

fn media_tab(ui: &mut Ui, st: &mut UiState, audio_only: bool) {
    // Зона импорта: клик откроет системный диалог, drag&drop работает по кадру.
    import_drop_zone(ui, st);
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new("МЕДИАТЕКА")
            .small()
            .color(Palette::TEXT_FAINT),
    );

    if !st
        .doc
        .assets
        .iter()
        .any(|asset| !audio_only || asset.kind == AssetKind::Audio)
    {
        empty_library_state(ui);
    } else {
        egui::ScrollArea::vertical().show(ui, |ui| asset_list(ui, st, audio_only));
    }
}
fn import_drop_zone(ui: &mut Ui, st: &mut UiState) {
    let width = ui.available_width();
    let height = 64.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());

    let hovered = resp.hovered();
    let stroke = Stroke::new(
        1.4,
        if hovered {
            Palette::ACCENT
        } else {
            Palette::STROKE
        },
    );
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(8),
        Color32::TRANSPARENT,
        stroke,
        egui::StrokeKind::Inside,
    );

    // Иконка плюс
    let icon_size = 20.0;
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + 22.0),
        Vec2::splat(icon_size),
    );
    let icon_color = if hovered {
        Palette::ACCENT
    } else {
        Palette::TEXT_DIM
    };
    icons::Icon::Plus.paint(painter, icon_rect, icon_color);

    painter.text(
        egui::pos2(rect.center().x, rect.top() + 40.0),
        Align2::CENTER_CENTER,
        "Импортировать файлы",
        egui::TextStyle::Button.resolve(ui.style()),
        if hovered {
            Palette::TEXT
        } else {
            Palette::TEXT_DIM
        },
    );
    painter.text(
        egui::pos2(rect.center().x, rect.bottom() - 10.0),
        Align2::CENTER_CENTER,
        "или перетащите сюда",
        egui::TextStyle::Small.resolve(ui.style()),
        Palette::TEXT_FAINT,
    );
    if resp.clicked() {
        st.want_import = true;
    }
    resp.on_hover_text("Перетащите файлы или нажмите для выбора");
}

fn empty_library_state(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 120.0), Sense::hover());
    let painter = ui.painter();
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.center().y - 14.0),
        Vec2::splat(28.0),
    );
    icons::Icon::Film.paint(painter, icon_rect, Palette::TEXT_FAINT);
    painter.text(
        egui::pos2(rect.center().x, rect.center().y + 18.0),
        Align2::CENTER_CENTER,
        "Здесь будут ваши клипы",
        egui::TextStyle::Small.resolve(ui.style()),
        Palette::TEXT_DIM,
    );
}

fn asset_list(ui: &mut Ui, st: &mut UiState, audio_only: bool) {
    // Снимок строк: ассет можно удалить/изменить, пока asset_row мутирует состояние.
    let rows: Vec<(Asset, Option<egui::TextureHandle>)> = st
        .doc
        .assets
        .iter()
        .filter(|asset| !audio_only || asset.kind == AssetKind::Audio)
        .map(|a| (a.clone(), st.thumbs.get(&a.id).cloned()))
        .collect();
    for (asset, thumb) in rows {
        asset_row(ui, &asset, thumb.as_ref(), st);
        ui.add_space(6.0);
    }
}

/// Строка ассета: двойной клик — добавить в таймлайн,
/// тянуть — перетащить на дорожку.
fn asset_row(ui: &mut Ui, asset: &Asset, thumb: Option<&egui::TextureHandle>, st: &mut UiState) {
    let width = ui.available_width();
    let height = 64.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());

    let bg = if resp.is_pointer_button_down_on() {
        Palette::SURFACE_PRESSED
    } else if resp.hovered() {
        Palette::SURFACE
    } else {
        Palette::PANEL_UPPER
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), bg);

    // Миниатюра: реальный кадр, если он снялся, иначе иконка-заглушка.
    let thumb_size = Vec2::new(86.0, height - 12.0);
    let thumb_rect =
        egui::Rect::from_min_size(egui::pos2(rect.left() + 6.0, rect.top() + 6.0), thumb_size);
    let painter = &ui.painter().with_clip_rect(rect);
    if let Some(tex) = thumb {
        painter.image(
            tex.id(),
            thumb_rect,
            crate::util::cover_uv(tex.size_vec2(), thumb_rect),
            Color32::WHITE,
        );
    } else {
        let is_video = asset.kind == AssetKind::Video;
        painter.rect_filled(
            thumb_rect,
            CornerRadius::same(5),
            if is_video {
                Palette::VIDEO_CLIP
            } else {
                Palette::AUDIO_CLIP
            },
        );
        let icon = if is_video {
            icons::Icon::Film
        } else {
            icons::Icon::MusicNote
        };
        icon.paint(
            painter,
            egui::Rect::from_center_size(thumb_rect.center(), Vec2::splat(20.0)),
            Color32::from_white_alpha(200),
        );
    }

    // Название и метаданные
    painter.text(
        egui::pos2(thumb_rect.right() + 10.0, rect.top() + 14.0),
        Align2::LEFT_CENTER,
        asset.display_name.clone(),
        egui::TextStyle::Button.resolve(ui.style()),
        Palette::TEXT,
    );
    let mut meta = format!("{:.1} с", asset.duration);
    if asset.width > 0 {
        meta += &format!(" · {}×{}", asset.width, asset.height);
    }
    if asset.fps > 0.0 {
        meta += &format!(" · {:.0} к/с", asset.fps);
    }
    painter.text(
        egui::pos2(thumb_rect.right() + 10.0, rect.top() + 32.0),
        Align2::LEFT_CENTER,
        meta,
        egui::TextStyle::Small.resolve(ui.style()),
        Palette::TEXT_FAINT,
    );

    // Двойной клик — добавить в конец первой дорожки нужного типа.
    if resp.double_clicked() {
        let _ = st.add_asset_to_timeline(asset.id);
    }
    // Тянем строку — payload для дропа на дорожке таймлайна.
    if resp.dragged() {
        DragAndDrop::set_payload(
            ui.ctx(),
            AssetDragPayload {
                asset_id: asset.id,
                is_audio: asset.kind == AssetKind::Audio,
            },
        );
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.on_hover_text("Двойной клик — добавить в таймлайн, или перетащите на дорожку");
}
