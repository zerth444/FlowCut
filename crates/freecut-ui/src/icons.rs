//! Единая иконография: все иконки рисуются примитивами egui,
//! поэтому приложение не зависит от эмодзи-шрифтов и имеет один стиль.

use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};

use crate::theme::Palette;

/// Набор иконок.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Play,
    Pause,
    StepBack,
    StepForward,
    SkipToStart,
    SkipToEnd,
    Undo,
    Redo,
    Plus,
    Minus,
    Close,
    Magnet,
    MagnetOff,
    Volume,
    VolumeMuted,
    Film,
    MusicNote,
    ChevronDown,
    Fit,
    Split,
}

fn p(x: f32, y: f32) -> Pos2 {
    Pos2::new(x, y)
}

fn bar(painter: &Painter, x0: f32, y0: f32, x1: f32, y1: f32, color: Color32) {
    let (min, max) = if x0 <= x1 && y0 <= y1 {
        (p(x0, y0), p(x1, y1))
    } else {
        (p(x1.min(x0), y1.min(y0)), p(x0.max(x1), y0.max(y1)))
    };
    painter.rect_filled(Rect::from_min_max(min, max), CornerRadius::same(1), color);
}

fn tri(painter: &Painter, a: Pos2, b: Pos2, c: Pos2, color: Color32) {
    painter.add(Shape::convex_polygon(vec![a, b, c], color, Stroke::NONE));
}

fn arc(painter: &Painter, center: Pos2, r: f32, from: f64, to: f64, stroke: Stroke) {
    let steps = 12;
    let mut pts = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = from + (to - from) * (i as f64 / steps as f64);
        pts.push(p(
            center.x + r * t.cos() as f32,
            center.y + r * t.sin() as f32,
        ));
    }
    painter.add(Shape::line(pts, stroke));
}

impl Icon {
    /// Рисовать иконку в заданном прямоугольнике цветом `color`.
    pub fn paint(self, painter: &Painter, rect: Rect, color: Color32) {
        let x0 = rect.left();
        let x1 = rect.left() + rect.width() * 0.5;
        let x2 = rect.right();
        let y0 = rect.top();
        let y1 = rect.bottom();
        let ym = rect.center().y;
        let cx = rect.center().x;

        match self {
            Icon::Play => tri(painter, p(x0, y0), p(x0, y1), p(x2, ym), color),
            Icon::Pause => {
                let w = rect.width() * 0.28;
                bar(painter, x0, y0, x0 + w, y1, color);
                bar(painter, x2 - w, y0, x2, y1, color);
            }
            Icon::StepBack | Icon::SkipToStart => {
                let bar_w = if self == Icon::SkipToStart {
                    rect.width() * 0.14
                } else {
                    rect.width() * 0.20
                };
                bar(painter, x0, y0, x0 + bar_w, y1, color);
                let tx = rect.left() + bar_w + rect.width() * 0.14;
                tri(painter, p(x2, y0), p(x2, y1), p(tx, ym), color);
            }
            Icon::StepForward | Icon::SkipToEnd => {
                let bar_w = if self == Icon::SkipToEnd {
                    rect.width() * 0.14
                } else {
                    rect.width() * 0.20
                };
                bar(painter, x2 - bar_w, y0, x2, y1, color);
                let tx = rect.right() - bar_w - rect.width() * 0.14;
                tri(painter, p(x0, y0), p(x0, y1), p(tx, ym), color);
            }
            Icon::Undo | Icon::Redo => {
                let dir = if self == Icon::Undo { 1.0 } else { -1.0 };
                let scale = rect.width().min(rect.height()) / 20.0;
                let at = |x: f32, y: f32| p(cx + x * scale * dir, ym + y * scale);
                let stroke = Stroke::new(1.7, color);
                painter.add(Shape::line(
                    vec![
                        at(-7.0, -3.0),
                        at(1.0, -3.0),
                        at(4.0, -2.4),
                        at(6.0, -0.5),
                        at(7.0, 2.0),
                        at(7.0, 5.0),
                    ],
                    stroke,
                ));
                painter.add(Shape::line(
                    vec![at(-3.0, -7.0), at(-7.0, -3.0), at(-3.0, 1.0)],
                    stroke,
                ));
            }
            Icon::Plus => {
                bar(painter, x0, ym - 1.25, x2, ym + 1.25, color);
                bar(painter, cx - 1.25, y0, cx + 1.25, y1, color);
            }
            Icon::Minus => bar(painter, x0, ym - 1.25, x2, ym + 1.25, color),
            Icon::Close => {
                painter.line_segment(
                    [p(x0 + 1.0, y0 + 1.0), p(x2 - 1.0, y1 - 1.0)],
                    Stroke::new(1.8, color),
                );
                painter.line_segment(
                    [p(x2 - 1.0, y0 + 1.0), p(x0 + 1.0, y1 - 1.0)],
                    Stroke::new(1.8, color),
                );
            }
            Icon::Magnet | Icon::MagnetOff => {
                // Перевёрнутая подкова: две ножки вниз с дугой снизу.
                let lw = 2.0;
                let inset = rect.width() * 0.22;
                let fcx = cx - inset;
                let tcx = cx + inset;
                painter.line_segment([p(fcx, y0), p(fcx, ym)], Stroke::new(lw, color));
                painter.line_segment([p(tcx, y0), p(tcx, ym)], Stroke::new(lw, color));
                arc(
                    painter,
                    p(cx, ym),
                    inset,
                    0.0,
                    std::f64::consts::PI,
                    Stroke::new(lw, color),
                );
                bar(painter, fcx - 2.5, y0 - 1.0, fcx + 2.5, y0 + 3.5, color);
                bar(painter, tcx - 2.5, y0 - 1.0, tcx + 2.5, y0 + 3.5, color);
                if self == Icon::MagnetOff {
                    painter.line_segment(
                        [p(x0 + 1.0, y1 - 1.0), p(x2 - 1.0, y0 + 1.0)],
                        Stroke::new(2.0, Palette::DANGER),
                    );
                }
            }
            Icon::Volume | Icon::VolumeMuted => {
                let w = rect.width() * 0.42;
                tri(
                    painter,
                    p(x0, ym - rect.height() * 0.18),
                    p(x0, ym + rect.height() * 0.18),
                    p(x0 + w, y1),
                    color,
                );
                tri(
                    painter,
                    p(x0, ym - rect.height() * 0.18),
                    p(x0 + w, y1),
                    p(x0 + w, y0),
                    color,
                );
                if self == Icon::Volume {
                    let r = rect.width() * 0.2;
                    for k in 1..=2 {
                        let rr = r * k as f32 + rect.width() * 0.10;
                        arc(
                            painter,
                            p(x0 + w + 1.0, ym),
                            rr,
                            -std::f64::consts::FRAC_PI_3,
                            std::f64::consts::FRAC_PI_3,
                            Stroke::new(1.6, color),
                        );
                    }
                } else {
                    painter.line_segment(
                        [p(x0 + w + 5.0, y0 + 3.0), p(x2 - 1.0, y1 - 3.0)],
                        Stroke::new(2.0, Palette::DANGER),
                    );
                }
            }
            Icon::Film => {
                let r = CornerRadius::same(3);
                painter.rect(
                    rect,
                    r,
                    Color32::TRANSPARENT,
                    Stroke::new(1.6, color),
                    egui::StrokeKind::Inside,
                );
                let hole = rect.width() / 4.0;
                for i in 0..3 {
                    let hx = x0 + hole * (i as f32 + 0.5);
                    bar(painter, hx - 2.0, y0 + 3.0, hx + 2.0, y0 + 4.5, color);
                    bar(painter, hx - 2.0, y1 - 4.5, hx + 2.0, y1 - 3.0, color);
                }
                bar(painter, x0 + 5.0, ym - 3.0, x2 - 5.0, ym + 3.0, color);
            }
            Icon::MusicNote => {
                let head_r = rect.width() * 0.16;
                painter.add(Shape::line(
                    vec![
                        p(x2 - head_r, y1 - head_r),
                        p(x0 + head_r, ym - 2.0),
                        p(x0 + head_r, y0 + 2.0),
                    ],
                    Stroke::new(2.0, color),
                ));
                painter.circle_filled(p(x2 - head_r, y1 - head_r), head_r * 1.3, color);
                painter.circle_filled(p(x0 + head_r, y0 + 2.0), head_r * 0.6, color);
            }
            Icon::ChevronDown => {
                painter.add(Shape::line(
                    vec![
                        p(x0 + 1.0, ym - 2.0),
                        p(x1, y1 - 2.0),
                        p(x2 - 1.0, ym - 2.0),
                    ],
                    Stroke::new(1.8, color),
                ));
            }
            Icon::Fit => {
                let l = rect.width() * 0.3;
                let t = 1.0;
                painter.line_segment(
                    [p(x0 + t, y0 + t), p(x0 + t, y0 + l)],
                    Stroke::new(1.8, color),
                );
                painter.line_segment(
                    [p(x0 + t, y0 + t), p(x0 + l, y0 + t)],
                    Stroke::new(1.8, color),
                );
                painter.line_segment(
                    [p(x2 - t, y0 + t), p(x2 - l, y0 + t)],
                    Stroke::new(1.8, color),
                );
                painter.line_segment(
                    [p(x2 - t, y0 + t), p(x2 - t, y0 + l)],
                    Stroke::new(1.8, color),
                );
                painter.line_segment(
                    [p(x0 + t, y1 - t), p(x0 + l, y1 - t)],
                    Stroke::new(1.8, color),
                );
                painter.line_segment(
                    [p(x0 + t, y1 - t), p(x0 + t, y1 - l)],
                    Stroke::new(1.8, color),
                );
                painter.line_segment(
                    [p(x2 - t, y1 - t), p(x2 - l, y1 - t)],
                    Stroke::new(1.8, color),
                );
                painter.line_segment(
                    [p(x2 - t, y1 - t), p(x2 - t, y1 - l)],
                    Stroke::new(1.8, color),
                );
            }
            Icon::Split => {
                let unit = rect.width().min(rect.height()) / 20.0;
                let point = |x: f32, y: f32| p(cx + x * unit, ym + y * unit);
                let stroke = Stroke::new(1.7, color);
                painter.circle_stroke(point(-6.0, -5.0), 3.0 * unit, stroke);
                painter.circle_stroke(point(-6.0, 5.0), 3.0 * unit, stroke);
                painter.line_segment([point(-3.5, -3.5), point(8.0, 7.0)], stroke);
                painter.line_segment([point(-3.5, 3.5), point(8.0, -7.0)], stroke);
            }
        }
    }
}

/// Стилизованная кнопка-иконка: квадрат с плашкой на ховере.
/// Подсветка показывает кликабельность, даже когда действие пока недоступно.
pub fn icon_button(ui: &mut Ui, icon: Icon, size: Vec2, enabled: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());

    let bg = if resp.is_pointer_button_down_on() {
        Palette::SURFACE_PRESSED
    } else if resp.hovered() {
        if enabled {
            Palette::SURFACE_HOVER
        } else {
            Palette::SURFACE // недоступная кнопка всё равно подсвечивается
        }
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, CornerRadius::same(6), bg);
    }

    let color = if enabled {
        if resp.hovered() {
            Palette::TEXT
        } else {
            Palette::TEXT_DIM
        }
    } else {
        Palette::TEXT_FAINT
    };

    let pad = Vec2::new(size.x * 0.24, size.y * 0.26);
    icon.paint(ui.painter(), rect.shrink2(pad), color);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(if enabled {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::Default
        });
    }
    resp
}
