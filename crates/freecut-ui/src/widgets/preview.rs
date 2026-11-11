//! Центральная зона: превью плеера с контролами и таймкодом.

use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, Ui, Vec2};

use crate::{app::UiState, icons, theme::Palette};

pub fn show(ui: &mut Ui, st: &mut UiState) {
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(Palette::BG)
                .inner_margin(egui::Margin::same(12)),
        )
        .show(ui, |ui| {
            ui.vertical(|ui| {
                // Карточка превью занимает всё место, кроме полосы контролов.
                player_card(ui, st);
                controls_bar(ui, st);
            });
        });
}

fn player_card(ui: &mut Ui, st: &UiState) {
    let available = ui.available_size();
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(available.x, (available.y - 46.0).max(0.0)),
        Sense::hover(),
    );

    let painter = ui.painter_at(rect);
    painter.rect(
        rect,
        CornerRadius::same(10),
        Palette::BG_DEEP,
        Stroke::new(1.0, Palette::STROKE_SOFT),
        egui::StrokeKind::Inside,
    );

    if let Some(tex) = &st.preview_tex {
        let canvas_height = (rect.width() * 9.0 / 16.0).min(rect.height()) * 0.96;
        let canvas = Rect::from_center_size(
            rect.center(),
            Vec2::new(canvas_height * 16.0 / 9.0, canvas_height),
        );
        painter.rect_filled(canvas, 0, Color32::BLACK);
        let gain = st
            .clip_under_playhead()
            .map_or(1.0, |clip| clip.video_gain(st.playhead));
        draw_layer(&painter, canvas, tex, 1.0, gain);
        if let Some((incoming, mix, gain)) = &st.secondary_preview {
            draw_layer(&painter, canvas, incoming, *mix, *gain);
        }
    } else if st.duration() == 0.0 {
        // Пустое состояние: иконка и подсказка.
        let icon_rect = Rect::from_center_size(
            egui::pos2(rect.center().x, rect.center().y - 30.0),
            Vec2::splat(44.0),
        );
        icons::Icon::Film.paint(
            &painter,
            icon_rect,
            Color32::from_rgba_unmultiplied(90, 90, 100, 90),
        );
        painter.text(
            egui::pos2(rect.center().x, rect.center().y + 18.0),
            Align2::CENTER_CENTER,
            "Перетащите медиа в медиатеку, чтобы начать",
            egui::TextStyle::Small.resolve(ui.style()),
            Palette::TEXT_FAINT,
        );
        painter.text(
            egui::pos2(rect.center().x, rect.center().y + 36.0),
            Align2::CENTER_CENTER,
            "Превью появится сразу после первого клипа на таймлайне",
            egui::TextStyle::Small.resolve(ui.style()),
            Color32::from_rgba_unmultiplied(80, 80, 90, 80),
        );
    }
}

fn draw_layer(
    painter: &egui::Painter,
    canvas: Rect,
    tex: &egui::TextureHandle,
    opacity: f64,
    gain: f64,
) {
    let size = tex.size_vec2();
    let scale = (canvas.width() / size.x).min(canvas.height() / size.y);
    let frame = Rect::from_center_size(canvas.center(), size * scale);
    let alpha = (opacity * 255.0).round() as u8;
    let shade = (opacity * gain * 255.0).round() as u8;
    let black = Color32::from_black_alpha(alpha);
    for rect in [
        Rect::from_min_max(canvas.min, egui::pos2(canvas.right(), frame.top())),
        Rect::from_min_max(egui::pos2(canvas.left(), frame.bottom()), canvas.max),
        Rect::from_min_max(
            egui::pos2(canvas.left(), frame.top()),
            egui::pos2(frame.left(), frame.bottom()),
        ),
        Rect::from_min_max(
            egui::pos2(frame.right(), frame.top()),
            egui::pos2(canvas.right(), frame.bottom()),
        ),
    ] {
        if rect.is_positive() {
            painter.rect_filled(rect, 0, black);
        }
    }
    painter.image(
        tex.id(),
        frame,
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::from_rgba_premultiplied(shade, shade, shade, alpha),
    );
}
fn controls_bar(ui: &mut Ui, st: &mut UiState) {
    let height = 46.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);

    // Слева: таймкод.
    let cur = format_time(st.playhead);
    let dur = format_time(st.duration());
    painter.text(
        egui::pos2(rect.left() + 4.0, rect.center().y),
        Align2::LEFT_CENTER,
        format!("{cur}/{dur}"),
        egui::TextStyle::Small.resolve(ui.style()),
        Palette::TEXT_DIM,
    );

    // Центр: покадровый шаг, play/pause.
    let cx = rect.center().x;
    let btn_h = 30.0;
    let small = Vec2::new(30.0, btn_h);
    let big = Vec2::splat(32.0);
    let gap = 8.0;
    let total_w = small.x + gap + big.x + gap + small.x;
    let left = cx - total_w * 0.5;

    let step_back_rect =
        Rect::from_min_size(egui::pos2(left, rect.center().y - btn_h * 0.5), small);
    let play_rect = Rect::from_min_size(
        egui::pos2(left + small.x + gap, rect.center().y - big.y * 0.5),
        big,
    );
    let step_fwd_rect = Rect::from_min_size(
        egui::pos2(
            left + small.x + gap + big.x + gap,
            rect.center().y - btn_h * 0.5,
        ),
        small,
    );

    let tb_step_back = ui.interact(
        step_back_rect,
        ui.scope_id().with("preview_step_back"),
        Sense::click(),
    );
    tiny_button(ui, step_back_rect, icons::Icon::StepBack, &tb_step_back);
    if tb_step_back.clicked() {
        st.seek(st.playhead - st.frame_step());
    }

    let tb_play = ui.interact(
        play_rect,
        ui.scope_id().with("preview_play"),
        Sense::click(),
    );
    play_pause_button(ui, play_rect, st.playing, st.duration() > 0.0, &tb_play);
    if tb_play.clicked() {
        st.toggle_playback();
    }

    let tb_step_fwd = ui.interact(
        step_fwd_rect,
        ui.scope_id().with("preview_step_fwd"),
        Sense::click(),
    );
    tiny_button(ui, step_fwd_rect, icons::Icon::StepForward, &tb_step_fwd);
    if tb_step_fwd.clicked() {
        st.seek(st.playhead + st.frame_step());
    }

    // Формат холста совпадает с MP4.
    painter.text(
        egui::pos2(rect.right() - 4.0, rect.center().y),
        Align2::RIGHT_CENTER,
        "Кадр 16:9",
        egui::TextStyle::Small.resolve(ui.style()),
        Palette::TEXT_FAINT,
    );
}

fn tiny_button(ui: &mut Ui, rect: Rect, icon: icons::Icon, resp: &egui::Response) {
    let bg = if resp.is_pointer_button_down_on() {
        Palette::SURFACE_PRESSED
    } else if resp.hovered() {
        Palette::SURFACE
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, CornerRadius::same(6), bg);
    }
    icon.paint(
        ui.painter(),
        rect.shrink2(Vec2::new(rect.width() * 0.24, rect.height() * 0.26)),
        Palette::TEXT,
    );
}

fn play_pause_button(ui: &mut Ui, rect: Rect, playing: bool, enabled: bool, resp: &egui::Response) {
    let bg = if !enabled {
        Palette::SURFACE
    } else if resp.is_pointer_button_down_on() {
        Palette::ACCENT_PRESSED
    } else if resp.hovered() {
        Palette::ACCENT_HOVER
    } else {
        Palette::ACCENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), bg);

    let icon = if playing {
        icons::Icon::Pause
    } else {
        icons::Icon::Play
    };
    let color = if enabled {
        Palette::ACCENT_TEXT
    } else {
        Palette::TEXT_FAINT
    };
    icon.paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(15.0)),
        color,
    );
}

fn format_time(t: f64) -> String {
    let t = t.max(0.0);
    let min = (t / 60.0).floor() as u32;
    let sec = (t % 60.0).floor() as u32;
    let cs = ((t - t.floor()) * 100.0) as u32;
    format!("{min:02}:{sec:02}.{cs:02}")
}
