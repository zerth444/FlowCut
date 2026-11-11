//! Таймлайн: линейка времени, дорожки, плейхед, зум и прокрутка.

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, Pos2, Rect, Sense, Shape, Stroke, TextStyle, Ui,
    Vec2,
};

use freecut_core::TrackKind;

use crate::{
    app::{AssetDragPayload, TrimSide, TrimState, UiState},
    icons,
    theme::{Palette, alpha},
};

/// Общая высота таймлайна.
pub const PANEL_HEIGHT: f32 = 262.0;

const TOOLBAR_H: f32 = 36.0;
const RULER_H: f32 = 26.0;
const HEADER_W: f32 = 96.0;
const ROW_GAP: f32 = 8.0;
const MIN_PPS: f64 = 2.0;
const MAX_PPS: f64 = 2400.0;
const FIT_W: f32 = 76.0;
const ZOOM_BTN: f32 = 24.0;
const ZOOM_SLIDER_W: f32 = 120.0;

// ------------------------------------------------------------- show / frame --

pub fn show(ui: &mut Ui, st: &mut UiState) {
    egui::Panel::bottom("timeline_panel")
        .default_size(PANEL_HEIGHT)
        .min_size(214.0)
        .max_size(600.0)
        .frame(egui::Frame::new().fill(Palette::PANEL))
        .show_separator_line(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0; // ряды вплотную: линейка, дорожки и плейхед совпадают по y
            let zone = ui.available_rect_before_wrap();
            ui.vertical(|ui| {
                toolbar(ui, st);
                ruler_row(ui, st);
                tracks_and_interactions(ui, st);
            });
            if st.trim.is_some() && !ui.input(|i| i.pointer.primary_down()) {
                st.finish_trim();
            }
            if !ui.input(|i| i.pointer.primary_down()) {
                st.moving = None;
            }
            // Колесо и Ctrl+колесо работают по всей высоте таймлайна.
            wheel_scroll_and_zoom(ui, zone, st);
        });
}

// ------------------------------------------------------------------ toolbar --

fn toolbar(ui: &mut Ui, st: &mut UiState) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), TOOLBAR_H), Sense::hover());
    let c = rect.center().y;

    // Слева: split и магнит.
    let buttons_rect = Rect::from_min_size(
        Pos2::new(rect.left() + 8.0, c - 13.0),
        Vec2::new(145.0, 26.0),
    );
    let mut buttons = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("timeline_tools")
            .max_rect(buttons_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let split = icons::icon_button(
        &mut buttons,
        icons::Icon::Split,
        Vec2::new(28.0, 26.0),
        st.split_target().is_some(),
    );
    if split
        .on_hover_text("Разделить клип по плейхеду (S)")
        .clicked()
    {
        st.split_at_playhead();
    }
    magnet_toggle(&mut buttons, st);
    if buttons
        .add_enabled(st.selected_clip.is_some(), egui::Button::new("Удалить"))
        .clicked()
    {
        st.delete_selected(true);
    }

    // Справа (справа налево): «Подогнать», плюс, слайдер, минус.
    // Сплошные константные шаги, привязка к правому краю — ничего не «съезжает».
    let right = rect.right() - 10.0;
    let fit_rect = Rect::from_min_size(Pos2::new(right - FIT_W, c - 13.0), Vec2::new(FIT_W, 26.0));
    let fit = ui.interact(fit_rect, ui.scope_id().with("timeline_fit"), Sense::click());
    if fit.hovered() {
        ui.painter()
            .rect_filled(fit_rect, CornerRadius::same(6), Palette::SURFACE);
    }
    ui.painter().text(
        fit_rect.center(),
        Align2::CENTER_CENTER,
        "Подогнать",
        TextStyle::Small.resolve(ui.style()),
        if fit.hovered() {
            Palette::TEXT
        } else {
            Palette::TEXT_DIM
        },
    );
    if fit.clicked() {
        st.pps =
            ((rect.width() - HEADER_W) as f64 / st.duration().max(1.0)).clamp(MIN_PPS, MAX_PPS);
        st.scroll = 0.0;
    }
    fit.on_hover_text("Показать весь таймлайн");

    let plus_rect = Rect::from_min_size(
        Pos2::new(fit_rect.left() - 8.0 - ZOOM_BTN, c - 12.0),
        Vec2::splat(ZOOM_BTN),
    );
    let plus_resp = ui.interact(
        plus_rect,
        ui.scope_id().with("timeline_zoom_plus"),
        Sense::click(),
    );
    zoom_btn(ui, plus_rect, &plus_resp, icons::Icon::Plus);
    if plus_resp.clicked() {
        st.pps = (st.pps * 1.25).min(MAX_PPS);
    }

    let slider_rect = Rect::from_min_size(
        Pos2::new(plus_rect.left() - 8.0 - ZOOM_SLIDER_W, c - 6.0),
        Vec2::new(ZOOM_SLIDER_W, 12.0),
    );
    let slider = egui::Slider::new(&mut st.pps, MIN_PPS..=MAX_PPS)
        .logarithmic(true)
        .show_value(false);
    ui.put(slider_rect, slider);

    let minus_rect = Rect::from_min_size(
        Pos2::new(slider_rect.left() - 8.0 - ZOOM_BTN, c - 12.0),
        Vec2::splat(ZOOM_BTN),
    );
    let minus_resp = ui.interact(
        minus_rect,
        ui.scope_id().with("timeline_zoom_minus"),
        Sense::click(),
    );
    zoom_btn(ui, minus_rect, &minus_resp, icons::Icon::Minus);
    if minus_resp.clicked() {
        st.pps = (st.pps * 0.8).max(MIN_PPS);
    }
}

/// Квадратная кнопка зума со встроенной иконкой.
fn zoom_btn(ui: &mut Ui, rect: Rect, resp: &egui::Response, icon: icons::Icon) {
    let bg = if resp.is_pointer_button_down_on() {
        Palette::SURFACE_PRESSED
    } else if resp.hovered() {
        Palette::SURFACE
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, CornerRadius::same(5), bg);
    }
    icon.paint(
        ui.painter(),
        rect.shrink2(Vec2::splat(7.0)),
        Palette::TEXT_DIM,
    );
}

fn magnet_toggle(ui: &mut Ui, st: &mut UiState) {
    let magnet_on = st.doc.timeline.magnetic;
    let resp = icons::icon_button(
        ui,
        if magnet_on {
            icons::Icon::Magnet
        } else {
            icons::Icon::MagnetOff
        },
        Vec2::new(30.0, 26.0),
        true,
    )
    .on_hover_text(if magnet_on {
        "Магнит включён: привязка при перемещении и уплотнение при вставке"
    } else {
        "Магнит выключен"
    });
    if resp.clicked() {
        st.toggle_magnet();
    }
}

// ------------------------------------------------------------------- ruler --

fn ruler_row(ui: &mut Ui, st: &mut UiState) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), RULER_H), Sense::hover());

    // Зона заголовков — точно над колонкой заголовков дорожек.
    let header = Rect::from_min_max(
        Pos2::new(rect.left() + 8.0, rect.top() + 4.0),
        Pos2::new(rect.left() + HEADER_W - 4.0, rect.bottom() - 4.0),
    );
    ui.painter()
        .rect_filled(header, CornerRadius::same(7), Palette::PANEL_UPPER);
    ui.painter().text(
        Pos2::new(header.left() + 8.0, header.center().y),
        Align2::LEFT_CENTER,
        "Время",
        TextStyle::Button.resolve(ui.style()),
        Palette::TEXT_FAINT,
    );

    // Шкала начинается строго правее колонки заголовков.
    // Тот же lanes_left, что и у дорожек (rect.left() + HEADER_W).
    let lanes_left = rect.left() + HEADER_W;
    let lanes_rect = Rect::from_min_max(
        Pos2::new(lanes_left, rect.top()),
        Pos2::new(rect.right(), rect.bottom()),
    );
    let (pps, scroll) = (st.pps, st.scroll);

    let scrub = ui.interact(
        lanes_rect,
        ui.scope_id().with("timeline_scrub"),
        Sense::click_and_drag(),
    );
    if (scrub.dragged() || scrub.clicked())
        && let Some(pos) = scrub.interact_pointer_pos()
    {
        st.seek(x_to_time(pps, scroll, lanes_left, pos.x));
        st.playing = false;
    }

    let painter = ui.painter_at(rect);
    painter.rect_filled(lanes_rect, CornerRadius::same(0), Palette::PANEL_UPPER);

    let step = major_step(pps);
    let last_t = scroll + lanes_rect.width() as f64 / pps;
    let mut t = (scroll / step).floor() * step;
    if t < 0.0 {
        t = 0.0;
    }
    while t <= last_t {
        let x = time_to_x(pps, scroll, lanes_left, t);
        if x > lanes_rect.right() {
            break;
        }
        painter.line_segment(
            [
                Pos2::new(x, rect.bottom() - 7.0),
                Pos2::new(x, rect.bottom()),
            ],
            Stroke::new(1.0, Palette::STROKE),
        );
        painter.text(
            Pos2::new(x + 7.0, rect.top() + 4.0),
            Align2::LEFT_TOP,
            time_label(t, step),
            TextStyle::Small.resolve(ui.style()),
            Palette::TEXT_FAINT,
        );
        t += step;
    }

    let ph_x = time_to_x(pps, scroll, lanes_left, st.playhead);
    if ph_x >= lanes_rect.left() && ph_x <= lanes_rect.right() {
        painter.add(Shape::convex_polygon(
            vec![
                Pos2::new(ph_x - 6.0, rect.top() + 1.0),
                Pos2::new(ph_x + 6.0, rect.top() + 1.0),
                Pos2::new(ph_x, rect.top() + 10.0),
            ],
            Palette::PLAYHEAD,
            Stroke::NONE,
        ));
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(ph_x - 1.0, rect.top() + 3.0),
                Pos2::new(ph_x + 1.0, rect.bottom()),
            ),
            CornerRadius::same(0),
            Palette::PLAYHEAD,
        );
    }

    if scrub.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
}

// --------------------------------------------------------------- координаты --

fn time_to_x(pps: f64, scroll: f64, lanes_left: f32, t: f64) -> f32 {
    lanes_left + ((t - scroll) * pps) as f32
}

fn x_to_time(pps: f64, scroll: f64, lanes_left: f32, x: f32) -> f64 {
    scroll + ((x - lanes_left) as f64 / pps)
}

fn major_step(pps: f64) -> f64 {
    for step in [
        0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0,
    ] {
        if step * pps >= 78.0 {
            return step;
        }
    }
    600.0
}

fn time_label(t: f64, step: f64) -> String {
    let t = t.max(0.0);
    if step < 1.0 {
        let centiseconds = (t * 100.0).round() as u64;
        return format!(
            "{}:{:02}.{:02}",
            centiseconds / 6000,
            centiseconds / 100 % 60,
            centiseconds % 100
        );
    }
    let min = (t / 60.0).floor() as u32;
    let sec = (t - f64::from(min) * 60.0).floor() as u32;
    if min == 0 {
        format!("0:{sec:02}")
    } else {
        format!("{min}:{sec:02}")
    }
}

// ------------------------------------------------------------------ дорожки --

fn tracks_and_interactions(ui: &mut Ui, st: &mut UiState) {
    let (area, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    let painter = ui.painter_at(area);
    painter.rect_filled(area, CornerRadius::same(0), Palette::BG);

    let lanes_rect = Rect::from_min_max(
        Pos2::new(area.left() + HEADER_W, area.top()),
        Pos2::new(area.right(), area.bottom()),
    );

    // Снимок данных дорожек: рисуем по копии, но ст mutating UI-состояние.
    let snapshot: Vec<(
        freecut_core::Id,
        TrackKind,
        String,
        bool,
        Vec<freecut_core::Clip>,
    )> = st
        .doc
        .timeline
        .tracks
        .iter()
        .map(|t| (t.id, t.kind, t.name.clone(), t.muted, t.clips.clone()))
        .collect();

    let mut y = area.top();
    let is_empty = snapshot.is_empty();
    let requested_height: f32 = snapshot
        .iter()
        .map(|(_, kind, _, _, _)| match kind {
            TrackKind::Video => st.video_row_h,
            TrackKind::Audio => st.audio_row_h,
        })
        .sum();
    let joined = st
        .doc
        .timeline
        .tracks
        .windows(2)
        .filter(|pair| pair[0].kind == TrackKind::Video && pair[1].video_sound)
        .count();
    let gaps = ROW_GAP * (snapshot.len() - joined) as f32;
    let height_scale = ((area.height() - gaps - 9.0) / requested_height).clamp(0.0, 1.0);
    for (row_index, (track_id, kind, name, muted, clips)) in snapshot.into_iter().enumerate() {
        let row_h = match kind {
            TrackKind::Video => st.video_row_h,
            TrackKind::Audio => st.audio_row_h,
        } * height_scale;
        if y + row_h > area.bottom() {
            break;
        }
        let row_rect = Rect::from_min_max(
            Pos2::new(area.left(), y),
            Pos2::new(area.right(), y + row_h),
        );
        track_row(
            painter.clone(),
            row_rect,
            lanes_rect,
            track_id,
            kind,
            name,
            muted,
            &clips,
            st,
            ui,
        );
        y += row_h;
        let joined_next = st
            .doc
            .timeline
            .tracks
            .get(row_index + 1)
            .is_some_and(|next| kind == TrackKind::Video && next.video_sound);
        if joined_next {
            continue;
        }

        // Разделитель между строками: тяни, чтобы менять высоту дорожки.
        let divider_rect = Rect::from_min_max(
            Pos2::new(area.left(), y),
            Pos2::new(area.right(), y + ROW_GAP),
        );
        let resp = ui.interact(
            divider_rect,
            ui.scope_id().with(("row_divider", track_id)),
            Sense::drag(),
        );
        let active = resp.hovered() || resp.dragged();
        painter.line_segment(
            [
                Pos2::new(divider_rect.left() + 4.0, divider_rect.center().y),
                Pos2::new(divider_rect.right() - 4.0, divider_rect.center().y),
            ],
            Stroke::new(
                if active { 2.0 } else { 0.0 },
                if active {
                    Palette::ACCENT
                } else {
                    Color32::TRANSPARENT
                },
            ),
        );
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        if resp.dragged() {
            match kind {
                TrackKind::Video => {
                    st.video_row_h = (st.video_row_h + resp.drag_delta().y).clamp(40.0, 240.0);
                }
                TrackKind::Audio => {
                    st.audio_row_h = (st.audio_row_h + resp.drag_delta().y).clamp(28.0, 240.0);
                }
            }
        } else if resp.hovered() {
            resp.on_hover_text("Тяни, чтобы изменить высоту дорожки");
        }

        y += ROW_GAP;
    }

    if is_empty {
        painter.text(
            lanes_rect.center(),
            Align2::CENTER_CENTER,
            "Дорожек нет",
            TextStyle::Small.resolve(ui.style()),
            Palette::TEXT_FAINT,
        );
    }

    draw_playhead(&painter, area, lanes_rect, st);
    h_scrollbar(ui, &painter, area, st);
}

#[allow(clippy::too_many_arguments)]
fn track_row(
    painter: egui::Painter,
    row_rect: Rect,
    lanes_rect: Rect,
    track_id: freecut_core::Id,
    kind: TrackKind,
    name: String,
    muted: bool,
    clips: &[freecut_core::Clip],
    st: &mut UiState,
    ui: &mut Ui,
) {
    let tracks = &st.doc.timeline.tracks;
    let position = tracks.iter().position(|track| track.id == track_id);
    let joined_above = position.is_some_and(|index| {
        tracks[index].video_sound && index > 0 && tracks[index - 1].kind == TrackKind::Video
    });
    let joined_below = position.is_some_and(|index| {
        kind == TrackKind::Video && tracks.get(index + 1).is_some_and(|track| track.video_sound)
    });
    let lane_rect = Rect::from_min_max(
        Pos2::new(
            lanes_rect.left(),
            row_rect.top() + if joined_above { 0.0 } else { 5.0 },
        ),
        Pos2::new(
            row_rect.right(),
            row_rect.bottom() - if joined_below { 0.0 } else { 5.0 },
        ),
    );

    let lane_bg = match kind {
        TrackKind::Video => Palette::BG_DEEP,
        TrackKind::Audio => alpha(Palette::AUDIO_CLIP, 24),
    };
    painter.rect_filled(lane_rect, CornerRadius::same(7), lane_bg);
    painter.rect(
        lane_rect,
        CornerRadius::same(7),
        Color32::TRANSPARENT,
        Stroke::new(1.0, Palette::STROKE_SOFT),
        egui::StrokeKind::Inside,
    );

    draw_grid(&painter, lane_rect, st.pps, st.scroll);

    // Заголовок дорожки.
    let header_rect = Rect::from_min_max(
        Pos2::new(row_rect.left() + 8.0, row_rect.top() + 5.0),
        Pos2::new(row_rect.left() + HEADER_W - 4.0, row_rect.bottom() - 5.0),
    );
    painter.rect_filled(header_rect, CornerRadius::same(7), Palette::PANEL_UPPER);

    let icon = if kind == TrackKind::Video {
        icons::Icon::Film
    } else {
        icons::Icon::MusicNote
    };
    icon.paint(
        &painter,
        Rect::from_center_size(
            Pos2::new(header_rect.left() + 12.0, header_rect.top() + 15.0),
            Vec2::splat(14.0),
        ),
        icons_color(muted),
    );
    painter.text(
        Pos2::new(header_rect.left() + 24.0, header_rect.top() + 15.0),
        Align2::LEFT_CENTER,
        name,
        TextStyle::Button.resolve(ui.style()),
        Palette::TEXT_DIM,
    );

    // Кнопка mute: внизу заголовка.
    let mute_rect = Rect::from_center_size(
        Pos2::new(header_rect.left() + 12.0, header_rect.bottom() - 11.0),
        Vec2::splat(16.0),
    );
    let mute_resp = ui.interact(
        mute_rect,
        ui.scope_id().with(("timeline_mute", track_id)),
        Sense::click(),
    );
    let mute_icon = if muted {
        icons::Icon::VolumeMuted
    } else {
        icons::Icon::Volume
    };
    mute_icon.paint(
        &painter,
        mute_rect.shrink2(Vec2::splat(2.0)),
        if muted {
            Palette::TEXT_FAINT
        } else {
            Palette::TEXT_DIM
        },
    );
    if mute_resp.clicked() {
        st.toggle_track_mute(track_id);
    }
    mute_resp.on_hover_text(if muted {
        "Включить дорожку"
    } else {
        "Заглушить дорожку"
    });

    // Зона дорожки в целом: клик по пустому месту снимает выделение.
    // Регистрируется до клипов, чтобы клипы лежали «выше» и ловили клик первыми.
    let lane_resp = ui.interact(
        lane_rect,
        ui.scope_id().with(("timeline_lane", track_id)),
        Sense::click(),
    );

    // --- Приём перетащенного ассета из медиатеки. ---------------------------
    // Payload совпадает по типу дорожки; на ховере рисуем «призрак» вставки,
    // на отпускании кнопки вставляем клип магнитно.
    let payload = egui::DragAndDrop::payload::<AssetDragPayload>(ui.ctx());
    let dropped_asset = payload
        .filter(|_| {
            !st.doc
                .timeline
                .tracks
                .iter()
                .any(|track| track.id == track_id && track.video_sound)
        })
        .filter(|p| p.is_audio == (kind == TrackKind::Audio))
        .map(|p| p.asset_id);
    if let Some(asset_id) = dropped_asset {
        let (hover_pos, dragging, released) = ui.ctx().input(|i| {
            (
                i.pointer.hover_pos(),
                i.pointer.is_decidedly_dragging(),
                i.pointer.any_released(),
            )
        });
        let over_lane = hover_pos.is_some_and(|pos| lane_rect.contains(pos));
        let asset_dur = st
            .doc
            .asset(asset_id)
            .map(|a| a.duration.max(0.0))
            .unwrap_or_default();
        if let Some(pos) = hover_pos
            && over_lane
            && (dragging || released)
        {
            let t = x_to_time(
                st.pps,
                st.scroll,
                lanes_rect.left(),
                pos.x.max(lanes_rect.left()),
            );
            let Some((start, _idx)) = st.plan_insert(track_id, asset_id, t) else {
                return;
            };
            if released {
                if st.insert_clip(asset_id, track_id, t).is_some() {
                    ui.ctx().request_repaint();
                }
                egui::DragAndDrop::clear_payload(ui.ctx());
            } else if dragging {
                // Призрак: место, куда встанет клип при отпускании.
                let gx0 =
                    time_to_x(st.pps, st.scroll, lanes_rect.left(), start).max(lane_rect.left());
                let gx1 = time_to_x(st.pps, st.scroll, lanes_rect.left(), start + asset_dur)
                    .min(lane_rect.right());
                let ghost = Rect::from_min_max(
                    Pos2::new(gx0, lane_rect.top() + 2.0),
                    Pos2::new(gx1.max(gx0 + 2.0), lane_rect.bottom() - 2.0),
                );
                painter.rect(
                    ghost,
                    CornerRadius::same(5),
                    alpha(Palette::ACCENT, 48),
                    Stroke::new(2.0, alpha(Palette::ACCENT, 190)),
                    egui::StrokeKind::Inside,
                );
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            }
        }
    }

    // Клипы дорожки.
    let (pps, scroll) = (st.pps, st.scroll);
    let mut clip_clicked = false;
    for (index, clip) in clips.iter().enumerate() {
        let x0 = time_to_x(pps, scroll, lanes_rect.left(), clip.track_start);
        let x1 = time_to_x(pps, scroll, lanes_rect.left(), clip.end());
        if x1 < lane_rect.left() + 2.0 || x0 > lane_rect.right() - 2.0 {
            continue; // целиком за пределами видимой полосы
        }
        let clip_rect = Rect::from_min_max(
            Pos2::new(
                x0.max(lane_rect.left() + 1.0),
                lane_rect.top() + if joined_above { 0.0 } else { 2.0 },
            ),
            Pos2::new(
                x1.min(row_rect.right() - 2.0),
                lane_rect.bottom() - if joined_below { 0.0 } else { 2.0 },
            ),
        );

        let fill = match kind {
            TrackKind::Video => Palette::VIDEO_CLIP,
            TrackKind::Audio => Palette::AUDIO_CLIP,
        };

        // Тело клипа со скруглением; миниатюра не вылезает за скруглённые углы.
        let thumb_painter = painter.with_clip_rect(clip_rect);
        thumb_painter.rect_filled(clip_rect, CornerRadius::same(5), fill);

        // Кадр-миниатюра слева наполном-высоте клипа, без искажений (обложка).
        let mut strip_w = 0.0_f32;
        if kind == TrackKind::Video
            && let Some(tex) = st.thumbs.get(&clip.asset_id)
        {
            let size = tex.size_vec2();
            if size.x > 0.0 && size.y > 0.0 {
                let h = clip_rect.height();
                strip_w = (h * size.x / size.y).min(clip_rect.width());
                let strip = Rect::from_min_max(
                    clip_rect.min,
                    Pos2::new(clip_rect.left() + strip_w, clip_rect.max.y),
                );
                thumb_painter.image(
                    tex.id(),
                    strip,
                    crate::util::cover_uv(size, strip),
                    Color32::WHITE,
                );
            }
        }
        if kind == TrackKind::Audio
            && let Some(tex) = st.waves.get(&clip.asset_id)
        {
            let asset_duration = st.asset_duration(clip.asset_id);
            if asset_duration > 0.0 {
                let to_uv = |x: f32| {
                    let local = f64::from(x - x0) / pps;
                    ((clip.source_in + local * clip.speed) / asset_duration).clamp(0.0, 1.0) as f32
                };
                let height = if kind == TrackKind::Video {
                    18.0
                } else {
                    clip_rect.height()
                };
                let wave_rect = Rect::from_min_max(
                    Pos2::new(clip_rect.left(), clip_rect.bottom() - height),
                    clip_rect.max,
                );
                let uv = Rect::from_min_max(
                    Pos2::new(to_uv(clip_rect.left()), 0.0),
                    Pos2::new(to_uv(clip_rect.right()), 1.0),
                );
                thumb_painter.image(tex.id(), wave_rect, uv, Color32::from_white_alpha(170));
            }
        }

        let selected = st.is_selected(clip.id);
        let hovered = ui
            .ctx()
            .pointer_hover_pos()
            .is_some_and(|pos| clip_rect.contains(pos));

        // Тело клипа регистрируется ПЕРВЫМ: позже зарегистрированные зоны
        // (края для тримминга) получают приоритет в egui-хит-тесте.
        let linked = clip.audio_parent.is_some();
        let body = ui.interact(
            clip_rect,
            ui.scope_id().with(("timeline_clip", clip.id)),
            if linked {
                Sense::click()
            } else {
                Sense::click_and_drag()
            },
        );
        if body.clicked() {
            body.surrender_focus();
            st.select_clip(clip.id, ui.input(|input| input.modifiers));
            clip_clicked = true;
        }
        if body.drag_started() {
            st.select_only(clip.id);
            if let Some(origin) = ui.input(|input| input.pointer.press_origin()) {
                let grab = x_to_time(pps, scroll, lanes_rect.left(), origin.x) - clip.track_start;
                st.moving = Some((clip.id, grab));
            }
        }
        if body.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            if let Some((id, grab)) = st.moving
                && id == clip.id
                && let Some(pos) = body.interact_pointer_pos()
            {
                let candidates: Vec<_> =
                    clips.iter().filter(|item| item.id != id).cloned().collect();
                let target =
                    x_to_time(pps, scroll, lanes_rect.left(), pos.x) - grab + clip.duration() * 0.5;
                let (start, _) = st.plan_move(&candidates, clip.duration(), target);
                let left = time_to_x(pps, scroll, lanes_rect.left(), start);
                let right = time_to_x(pps, scroll, lanes_rect.left(), start + clip.duration());
                let ghost = Rect::from_min_max(
                    Pos2::new(left, clip_rect.top()),
                    Pos2::new(right, clip_rect.bottom()),
                )
                .intersect(lane_rect);
                if ghost.is_positive() {
                    painter.rect(
                        ghost,
                        5,
                        alpha(Palette::ACCENT, 45),
                        Stroke::new(1.5, Palette::ACCENT),
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }
        if st.trim.is_none()
            && let Some((id, grab)) = st.moving
            && id == clip.id
            && ui.input(|input| input.pointer.primary_released())
            && let Some(pos) = ui.input(|input| input.pointer.latest_pos())
        {
            st.moving = None;
            let time = x_to_time(pps, scroll, lanes_rect.left(), pos.x);
            st.move_clip(clip.id, time - grab + clip.duration() * 0.5);
        }
        if body.hovered() && st.trim.is_none() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            let asset_name = st
                .doc
                .asset(clip.asset_id)
                .map(|a| a.display_name.clone())
                .unwrap_or_default();
            if !asset_name.is_empty() {
                body.on_hover_text(asset_name.as_str());
            }
        }

        // Тримминг: края с зажимаемыми «краешками»; registered поверх тела.
        let asset_dur = st.asset_duration(clip.asset_id);
        let asset_name = st
            .doc
            .asset(clip.asset_id)
            .map(|a| a.display_name.clone())
            .unwrap_or_default();
        let edge_w = 7.0_f32.min(clip_rect.width() * 0.35);
        for (side, cursor) in [
            (TrimSide::Left, CursorIcon::ResizeWest),
            (TrimSide::Right, CursorIcon::ResizeEast),
        ] {
            if linked {
                break;
            }
            let trimming = st
                .trim
                .is_some_and(|t| t.clip_id == clip.id && t.side == side);
            if !trimming
                && ((side == TrimSide::Left && x0 < lane_rect.left())
                    || (side == TrimSide::Right && x1 > lane_rect.right()))
            {
                continue;
            }
            let side_rect = match side {
                TrimSide::Left => Rect::from_min_max(
                    clip_rect.min,
                    Pos2::new(clip_rect.left() + edge_w, clip_rect.max.y),
                ),
                TrimSide::Right => Rect::from_min_max(
                    Pos2::new(clip_rect.right() - edge_w, clip_rect.min.y),
                    clip_rect.max,
                ),
            };
            let resp = ui.interact(
                side_rect,
                ui.scope_id().with(("clip_edge", clip.id, side)),
                Sense::click_and_drag(),
            );
            if resp.hovered() || trimming {
                ui.ctx().set_cursor_icon(cursor);
            }
            if resp.clicked() {
                resp.surrender_focus();
                st.select_clip(clip.id, ui.input(|input| input.modifiers));
            }
            if resp.drag_started()
                && !trimming
                && let Some(pos) = resp.interact_pointer_pos()
            {
                let origin_x = ui.input(|i| i.pointer.press_origin().map_or(pos.x, |p| p.x));
                st.begin_trim(TrimState {
                    clip_id: clip.id,
                    side,
                    anchor_t: x_to_time(pps, scroll, lanes_rect.left(), origin_x),
                    orig_track_start: clip.track_start,
                    orig_source_in: clip.source_in,
                    orig_source_duration: clip.source_duration,
                    speed: clip.speed,
                    min_start: index.checked_sub(1).map_or(0.0, |prev| clips[prev].end()),
                    max_end: clips
                        .get(index + 1)
                        .map_or(f64::INFINITY, |next| next.track_start),
                });
                clip_clicked = true;
            }
            if trimming || resp.drag_started() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    apply_trim(
                        st,
                        clip.id,
                        side,
                        pos.x,
                        pps,
                        scroll,
                        lanes_rect.left(),
                        asset_dur,
                    );
                }
                if resp.drag_stopped() {
                    st.finish_trim();
                }
            }
            // Подсветка края при наведении/тримме: видимые «ручки».
            if hovered || selected || trimming {
                let grip_color = if trimming {
                    Palette::ACCENT
                } else {
                    alpha(Color32::WHITE, 140)
                };
                match side {
                    TrimSide::Left => {
                        thumb_painter.rect_filled(
                            Rect::from_min_max(
                                Pos2::new(clip_rect.left() + 2.0, clip_rect.min.y + 3.0),
                                Pos2::new(clip_rect.left() + 4.5, clip_rect.max.y - 3.0),
                            ),
                            CornerRadius::same(2),
                            grip_color,
                        );
                    }
                    TrimSide::Right => {
                        thumb_painter.rect_filled(
                            Rect::from_min_max(
                                Pos2::new(clip_rect.right() - 4.5, clip_rect.min.y + 3.0),
                                Pos2::new(clip_rect.right() - 2.0, clip_rect.max.y - 3.0),
                            ),
                            CornerRadius::same(2),
                            grip_color,
                        );
                    }
                }
            }
        }

        // Контур клипа поверх миниатюры (и выделение).
        if selected {
            painter.rect(
                clip_rect,
                CornerRadius::same(5),
                Color32::TRANSPARENT,
                Stroke::new(2.0, Palette::ACCENT),
                egui::StrokeKind::Inside,
            );
        } else {
            painter.rect(
                clip_rect,
                CornerRadius::same(5),
                Color32::TRANSPARENT,
                Stroke::new(1.0, alpha(Color32::BLACK, 90)),
                egui::StrokeKind::Inside,
            );
        }

        // Имя ассета: правее миниатюры, компактно, не перекрывает картинку.
        let name_x = clip_rect.left() + strip_w + 6.0;
        if !asset_name.is_empty() && clip_rect.right() - name_x > 56.0 {
            thumb_painter.text(
                Pos2::new(name_x, clip_rect.bottom() - 10.0),
                Align2::LEFT_CENTER,
                asset_name.as_str(),
                TextStyle::Small.resolve(ui.style()),
                alpha(Color32::WHITE, 220),
            );
        }
    }

    if lane_resp.clicked() && !clip_clicked {
        st.clear_selection();
    }
}

/// Применить перетаскивание края клипа (тримминг): меняет track_start/source_in
/// или source_duration с ограничениями по границам ассета; дорожка уплотняется
/// магнитно после каждого шага.
#[allow(clippy::too_many_arguments)]
fn apply_trim(
    st: &mut UiState,
    clip_id: freecut_core::Id,
    side: TrimSide,
    pointer_x: f32,
    pps: f64,
    scroll: f64,
    lanes_left: f32,
    asset_dur: f64,
) {
    let Some(trim) = st.trim else {
        return;
    };
    if trim.clip_id != clip_id {
        return;
    }
    let min_content = 0.05_f64.min(trim.orig_source_duration);
    let t = x_to_time(pps, scroll, lanes_left, pointer_x);
    let dt = t - trim.anchor_t;
    let speed = trim.speed.max(0.01);

    match side {
        TrimSide::Left => {
            // Сдвиг левого края = сдвиг внутрь источника на dt·speed.
            let lower =
                (trim.orig_source_in + (trim.min_start - trim.orig_track_start) * speed).max(0.0);
            let upper = trim.orig_source_in + trim.orig_source_duration - min_content;
            let source_in = (trim.orig_source_in + dt * speed).clamp(lower.min(upper), upper);
            let consumed = source_in - trim.orig_source_in;
            let new_start = trim.orig_track_start + consumed / speed;
            st.update_clip(clip_id, |clip| {
                clip.track_start = new_start;
                clip.source_in = source_in;
                clip.source_duration = trim.orig_source_duration - consumed;
            });
        }
        TrimSide::Right => {
            // Сдвиг правого края: длительность содержимого + dt·speed.
            let mut max_duration = asset_dur - trim.orig_source_in;
            if !st.doc.timeline.magnetic {
                max_duration = max_duration.min((trim.max_end - trim.orig_track_start) * speed);
            }
            let source_duration = (trim.orig_source_duration + dt * speed)
                .clamp(min_content, max_duration.max(min_content));
            st.update_clip(clip_id, |clip| {
                clip.source_duration = source_duration;
            });
        }
    }
    // Магнит: зазоров не оставляем, последующие клипы подъезжают.
    if side == TrimSide::Right {
        st.repack_track(clip_id);
    }
}

fn icons_color(muted: bool) -> Color32 {
    if muted {
        Palette::TEXT_FAINT
    } else {
        Palette::TEXT_DIM
    }
}

fn draw_grid(painter: &egui::Painter, lane_rect: Rect, pps: f64, scroll: f64) {
    let step = major_step(pps);
    let last_t = scroll + lane_rect.width() as f64 / pps;
    let mut t = (scroll / step).floor() * step;
    if t < 0.0 {
        t = 0.0;
    }
    while t <= last_t {
        let x = time_to_x(pps, scroll, lane_rect.left(), t);
        if x > lane_rect.right() {
            break;
        }
        painter.line_segment(
            [
                Pos2::new(x, lane_rect.top()),
                Pos2::new(x, lane_rect.bottom()),
            ],
            Stroke::new(1.0, alpha(Color32::WHITE, 7)),
        );
        t += step;
    }
}

// -------------------------------------------------------------- скролл/зум --

fn wheel_scroll_and_zoom(ui: &mut Ui, rect: Rect, st: &mut UiState) {
    let lanes_left = rect.left() + HEADER_W;
    // Важно: при Ctrl egui зануляет smooth_scroll_delta и отдаёт зум
    // мультипликатором через zoom_delta() (см. egui 0.37 input_state/mod.rs).
    let (ctrl, zoom, dy, dx, pointer_in) = ui.ctx().input(|i| {
        (
            i.modifiers.ctrl,
            i.zoom_delta(),
            i.smooth_scroll_delta.y,
            i.smooth_scroll_delta.x,
            i.pointer.hover_pos().is_some_and(|pos| rect.contains(pos)),
        )
    });
    if !pointer_in {
        return;
    }
    if ctrl {
        if (zoom - 1.0).abs() < 1e-4 {
            return; // Ctrl зажат, но колесо не крутится
        }
        if let Some(pos) = ui.ctx().input(|i| i.pointer.hover_pos()) {
            // Зум вокруг точки под курсором (та же шкала, что у ползунка).
            let anchor_x = pos.x.max(lanes_left); // над колонкой заголовков зум тоже работает
            let t = x_to_time(st.pps, st.scroll, lanes_left, anchor_x);
            let factor = f64::from(zoom).clamp(0.75, 1.5);
            st.pps = (st.pps * factor).clamp(MIN_PPS, MAX_PPS);
            st.scroll = (t - (anchor_x - lanes_left) as f64 / st.pps).max(0.0);
        }
    } else if dy.abs() > 0.5 || dx.abs() > 0.5 {
        st.scroll = (st.scroll + f64::from(dy + dx) / st.pps).max(0.0);
    }
}

fn draw_playhead(painter: &egui::Painter, area: Rect, lanes_rect: Rect, st: &mut UiState) {
    let ph_x = time_to_x(st.pps, st.scroll, lanes_rect.left(), st.playhead);
    if ph_x < lanes_rect.left() - 1.0 || ph_x > lanes_rect.right() + 1.0 {
        return;
    }

    painter.rect_filled(
        Rect::from_min_max(
            Pos2::new(ph_x - 1.0, area.top()),
            Pos2::new(ph_x + 1.0, area.bottom()),
        ),
        CornerRadius::same(0),
        Palette::PLAYHEAD,
    );
}

fn h_scrollbar(ui: &mut Ui, painter: &egui::Painter, area: Rect, st: &mut UiState) {
    let bar = Rect::from_min_max(
        Pos2::new(area.left() + HEADER_W + 4.0, area.bottom() - 8.0),
        Pos2::new(area.right() - 8.0, area.bottom() - 2.0),
    );
    painter.rect_filled(bar, CornerRadius::same(3), Palette::STROKE_SOFT);

    let lanes_w = bar.width() as f64;
    let content_w = ((st.duration().max(st.playhead).max(st.scroll) + 30.0) * st.pps).max(lanes_w);
    let visible = (lanes_w / content_w).clamp(0.08, 1.0);
    let max_scroll = (content_w - lanes_w) / st.pps;
    let thumb_w = (lanes_w * visible) as f32;
    let frac = if max_scroll > 0.0 {
        (st.scroll / max_scroll).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let thumb_x = bar.left() + (bar.width() - thumb_w) * frac as f32;

    painter.rect_filled(
        Rect::from_min_max(
            Pos2::new(thumb_x, bar.top()),
            Pos2::new(thumb_x + thumb_w, bar.bottom()),
        ),
        CornerRadius::same(3),
        Palette::STROKE,
    );

    let resp = ui.interact(
        bar,
        ui.scope_id().with("timeline_h_scrollbar"),
        Sense::click_and_drag(),
    );
    if (resp.dragged() || resp.clicked())
        && max_scroll > 0.0
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let f = ((pos.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
        st.scroll = f as f64 * max_scroll;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use egui::{Event, Modifiers, PointerButton, RawInput};
    use freecut_core::{Asset, AssetKind};

    #[test]
    fn free_trim_moves_left_edge_and_keeps_right_edge() {
        let mut st = UiState::new();
        let mut asset = Asset::new(AssetKind::Video, "test.mp4".into());
        asset.duration = 5.0;
        let id = asset.id;
        st.doc.assets.push(asset);
        let track = st.doc.timeline.tracks[0].id;
        let clip_id = st.insert_clip(id, track, f64::INFINITY).unwrap();
        st.move_clip(clip_id, 7.5);
        let clip = st.doc.timeline.find_clip(clip_id).unwrap().clone();
        assert_eq!(clip.track_start, 5.0);
        st.begin_trim(TrimState {
            clip_id,
            side: TrimSide::Left,
            anchor_t: 5.0,
            orig_track_start: clip.track_start,
            orig_source_in: clip.source_in,
            orig_source_duration: clip.source_duration,
            speed: clip.speed,
            min_start: 0.0,
            max_end: f64::INFINITY,
        });
        apply_trim(
            &mut st,
            clip_id,
            TrimSide::Left,
            600.0,
            100.0,
            0.0,
            0.0,
            5.0,
        );
        st.finish_trim();
        let clip = st.doc.timeline.find_clip(clip_id).unwrap();
        assert_eq!(clip.track_start, 6.0);
        assert_eq!(clip.end(), 10.0);
        st.seek(30.0);
        assert_eq!(st.playhead, 30.0);
        assert!(st.clip_under_playhead().is_none());
    }

    #[test]
    fn left_handle_drag_moves_left_edge_with_magnet_enabled() {
        let mut st = UiState::new();
        st.doc.timeline.magnetic = true;
        let mut asset = Asset::new(AssetKind::Video, "test.mp4".into());
        asset.duration = 5.0;
        let id = asset.id;
        st.doc.assets.push(asset);
        let track = st.doc.timeline.tracks[0].id;
        let _ = st.insert_clip(id, track, f64::INFINITY);
        let _ = st.insert_clip(id, track, f64::INFINITY);
        let ctx = egui::Context::default();
        let row = Rect::from_min_size(Pos2::new(0.0, 40.0), Vec2::new(1000.0, 74.0));
        let lanes = Rect::from_min_max(Pos2::new(HEADER_W, row.top()), row.max);
        let start = Pos2::new(HEADER_W + 3.0, 70.0);
        let end = start + Vec2::new(st.pps as f32, 0.0);
        let pointer = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        for events in [
            vec![Event::PointerMoved(start)],
            vec![pointer(start, true)],
            vec![Event::PointerMoved(end)],
            vec![pointer(end, false)],
        ] {
            let clips = st.doc.timeline.tracks[0].clips.clone();
            ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 200.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    track_row(
                        ui.painter_at(row),
                        row,
                        lanes,
                        track,
                        TrackKind::Video,
                        "Видео".into(),
                        false,
                        &clips,
                        &mut st,
                        ui,
                    );
                    if st.trim.is_some() && !ui.input(|input| input.pointer.primary_down()) {
                        st.finish_trim();
                    }
                },
            )
            .textures_delta
            .clear();
        }
        let clips = &st.doc.timeline.tracks[0].clips;
        assert!((clips[0].source_in - 1.0).abs() < 1e-6, "{:?}", clips[0]);
        assert!((clips[0].track_start - 1.0).abs() < 1e-6);
        assert!((clips[0].end() - 5.0).abs() < 1e-6);
        assert!((clips[1].track_start - 5.0).abs() < 1e-6);
        st.undo();
        assert_eq!(st.doc.timeline.tracks[0].clips[0].source_in, 0.0);
        st.redo();
        assert!((st.doc.timeline.tracks[0].clips[0].source_in - 1.0).abs() < 1e-6);
    }
}
