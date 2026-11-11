//! Правая панель: инспектор выбранного клипа — громкость, fade, скорость.
//! Изменения клипа применяются к превью и экспорту.

use egui::{Align2, Sense, Ui, Vec2};

use freecut_core::{Fade, Id};

use crate::{app::UiState, icons, theme::Palette, widgets};

/// Ширина правой панели.
pub const PANEL_WIDTH: f32 = 248.0;

pub fn show(ui: &mut Ui, st: &mut UiState) {
    egui::Panel::right("inspector_panel")
        .default_size(PANEL_WIDTH)
        .min_size(180.0)
        .max_size(420.0)
        .frame(
            egui::Frame::new()
                .fill(Palette::PANEL)
                .inner_margin(egui::Margin::same(12)),
        )
        .show_separator_line(false)
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.label(section_title());
                ui.add_space(8.0);

                let clip = st
                    .selected_clip
                    .and_then(|id| st.doc.timeline.find_clip(id))
                    .cloned();

                egui::ScrollArea::vertical().show(ui, |ui| match clip {
                    None => no_selection(ui),
                    Some(clip) => clip_sections(ui, st, clip.id, &clip),
                });
            });
        });
}

fn section_title() -> egui::RichText {
    crate::theme::section_title("Инспектор")
}
fn no_selection(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 110.0), Sense::hover());
    ui.painter().text(
        egui::pos2(rect.center().x, rect.center().y - 8.0),
        Align2::CENTER_CENTER,
        "Клип не выбран",
        egui::TextStyle::Button.resolve(ui.style()),
        Palette::TEXT_DIM,
    );
    ui.painter().text(
        egui::pos2(rect.center().x, rect.center().y + 12.0),
        Align2::CENTER_CENTER,
        "Выберите клип на таймлайне",
        egui::TextStyle::Small.resolve(ui.style()),
        Palette::TEXT_FAINT,
    );
}

fn clip_sections(ui: &mut Ui, st: &mut UiState, clip_id: Id, clip: &freecut_core::Clip) {
    let Some(asset) = st.doc.asset(clip.asset_id) else {
        return;
    };
    let video = st
        .doc
        .timeline
        .video_tracks()
        .any(|track| track.clips.iter().any(|item| item.id == clip_id));
    let audio = asset.has_audio && !clip.separate_audio;
    let audio_clip_id = clip.audio_parent.unwrap_or(clip_id);
    if clip.audio_parent.is_some() {
        ui.label("Звук следует за видео при перемещении и обрезке.");
        if ui.button("Отвязать звук от видео").clicked() {
            st.detach_audio(clip_id);
        }
    }
    widgets::card(ui, |ui| {
        ui.label(
            egui::RichText::new("Диапазон источника")
                .small()
                .color(Palette::TEXT_DIM),
        );
        ui.label(format!(
            "{:.2} с – {:.2} с",
            clip.source_in,
            clip.source_in + clip.source_duration
        ));
        ui.add_space(8.0);
        if clip.audio_parent.is_none() {
            section(ui, "Скорость");
            speed_row(ui, st, clip_id, clip.speed);
        }
        if audio {
            ui.add_space(8.0);
            section(ui, "Громкость");
            volume_row(ui, st, audio_clip_id, clip.volume, clip.muted);
            section(ui, "Затухание звука");
            fade_row(ui, st, audio_clip_id, "Вход", clip.audio_fade, |clip| {
                &mut clip.audio_fade
            });
            fade_row(
                ui,
                st,
                audio_clip_id,
                "Выход",
                clip.audio_fade_out,
                |clip| &mut clip.audio_fade_out,
            );
        }
    });
    if video {
        ui.add_space(6.0);
        widgets::card(ui, |ui| {
            section(ui, "Затемнение видео");
            fade_row(ui, st, clip_id, "Вход", clip.video_fade, |clip| {
                &mut clip.video_fade
            });
            fade_row(ui, st, clip_id, "Выход", clip.video_fade_out, |clip| {
                &mut clip.video_fade_out
            });
            ui.add_space(8.0);
            section(ui, "Цвет");
            let mut filters = clip.filters;
            let brightness = slider_row(ui, "Яркость", &mut filters.brightness, -1.0..=1.0);
            let contrast = slider_row(ui, "Контраст", &mut filters.contrast, 0.0..=2.0);
            let saturation = slider_row(ui, "Насыщенность", &mut filters.saturation, 0.0..=2.0);
            if brightness || contrast || saturation {
                st.update_clip(clip_id, |clip| clip.filters = filters);
            }
            if st.doc.timeline.magnetic && st.has_next_clip(clip_id) {
                ui.add_space(8.0);
                let mut transition = clip.transition_out.unwrap_or_default();
                let mut enabled = clip.transition_out.is_some();
                if ui
                    .checkbox(&mut enabled, "Кросс-диссолв со следующим")
                    .changed()
                {
                    st.set_transition(clip_id, enabled.then_some(transition));
                }
                if enabled
                    && ui
                        .add(
                            egui::DragValue::new(&mut transition.duration)
                                .range(0.05..=3.0)
                                .speed(0.05)
                                .suffix(" с"),
                        )
                        .changed()
                {
                    st.set_transition(clip_id, Some(transition));
                }
            }
        });
    }
}
fn section(ui: &mut Ui, title: &str) {
    ui.label(
        egui::RichText::new(title)
            .small()
            .color(Palette::TEXT_FAINT),
    );
}

/// Живая громкость: мьют-кнопка + слайдер; изменение сразу в клипе.
fn volume_row(ui: &mut Ui, st: &mut UiState, clip_id: Id, value: f64, muted: bool) {
    ui.horizontal(|ui| {
        let icon = if muted {
            icons::Icon::VolumeMuted
        } else {
            icons::Icon::Volume
        };
        let toggle = icons::icon_button(ui, icon, Vec2::new(24.0, 24.0), true);
        if toggle.clicked() {
            st.update_clip(clip_id, |clip| clip.muted = !clip.muted);
        }

        let mut volume = value;
        let slider = ui.add(egui::Slider::new(&mut volume, 0.0..=2.0).show_value(false));
        if slider.changed() {
            st.update_clip(clip_id, |clip| clip.volume = volume);
        }

        let label = if muted {
            "Мьют".to_owned()
        } else {
            format!("{}%", (value * 100.0).round())
        };
        ui.label(egui::RichText::new(label).small().color(Palette::TEXT_DIM));
    });
}

fn fade_row(
    ui: &mut Ui,
    st: &mut UiState,
    id: Id,
    label: &str,
    mut fade: Fade,
    field: impl FnOnce(&mut freecut_core::Clip) -> &mut Fade,
) {
    ui.horizontal(|ui| {
        let toggled = ui.checkbox(&mut fade.enabled, label).changed();
        let adjusted = ui
            .add_enabled(
                fade.enabled,
                egui::DragValue::new(&mut fade.duration)
                    .range(0.0..=60.0)
                    .speed(0.05)
                    .fixed_decimals(2)
                    .suffix(" с"),
            )
            .changed();
        if toggled || adjusted {
            st.update_clip(id, |clip| *field(clip) = fade);
        }
    });
}
/// Живая скорость 0.25×..4×; видеопоток и аудио перезапускаются сами.
fn speed_row(ui: &mut Ui, st: &mut UiState, clip_id: Id, speed: f64) {
    let mut value = speed;
    if ui
        .add(egui::Slider::new(&mut value, 0.25..=4.0).suffix("×"))
        .changed()
    {
        st.update_clip(clip_id, |clip| clip.speed = value);
        st.repack_track(clip_id);
    }
}

fn slider_row(
    ui: &mut Ui,
    label: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
) -> bool {
    ui.label(label);
    ui.add(egui::Slider::new(value, range)).changed()
}
