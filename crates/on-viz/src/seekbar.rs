//! The progress bar of the transport: scrub anywhere on it, drag the A and B flags to
//! choose a section to practise, and see where the bars of the music fall.

use bevy_egui::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

/// How tall the bar is. Tall enough to hit without aiming.
const HEIGHT: f32 = 44.0;

/// Room kept free at either end of the track so the flags there can be grabbed.
const MARGIN: f32 = 12.0;

/// How close, in pixels, a dragged flag has to come to a bar line to snap to it.
const SNAP: f32 = 7.0;

/// The least width of a section, in seconds.
const MIN_SECTION: f64 = 0.25;

/// How much of a drag counts while Shift is held, for placing the playhead precisely.
const FINE: f64 = 0.1;

/// What the user did to the bar this frame.
#[derive(Default)]
pub struct Outcome {
    /// Where playback should jump to.
    pub seek: Option<f64>,
}

/// Where the bars fall, as a fraction of the duration, and how many to skip between the
/// ticks and numbers drawn so that they stay readable.
fn stride(bars: usize, width: f32, wanted: f32) -> usize {
    if bars == 0 {
        return 1;
    }
    ((wanted * bars as f32 / width.max(1.0)).ceil() as usize).max(1)
}

/// The number of the bar that is playing at `seconds`, counting from 1.
fn bar_at(bars: &[f64], seconds: f64) -> Option<usize> {
    let passed = bars.partition_point(|start| *start <= seconds + 1e-6);
    (passed > 0).then_some(passed)
}

/// Draws the bar and applies what is done to it. `section` is the A–B pair, with `B` at
/// infinity while only `A` is set.
pub fn show(
    ui: &mut egui::Ui,
    position: f64,
    duration: f64,
    bars: &[f64],
    section: &mut Option<(f64, f64)>,
    first: f64,
) -> Outcome {
    let mut outcome = Outcome::default();
    let width = ui.available_width().max(120.0);
    let (frame, bar) = ui.allocate_exact_size(Vec2::new(width, HEIGHT), Sense::click_and_drag());
    let track = Rect::from_min_max(
        Pos2::new(frame.left() + MARGIN, frame.top() + 14.0),
        Pos2::new(frame.right() - MARGIN, frame.bottom() - 6.0),
    );
    let span = (duration - first).max(1e-3);
    let x_of = |t: f64| track.left() + ((t - first) / span).clamp(0.0, 1.0) as f32 * track.width();
    let t_of = |x: f32| first + f64::from(((x - track.left()) / track.width()).clamp(0.0, 1.0)) * span;
    let snapped = |t: f64| {
        let x = x_of(t);
        bars.iter()
            .map(|start| (*start, (x_of(*start) - x).abs()))
            .filter(|(_, away)| *away <= SNAP)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(t, |(start, _)| start)
    };

    // The flags sit above the bar in the interaction order, so they win over scrubbing.
    let mut flag_dragged = false;
    let ends = section.map(|(start, end)| (start, if end.is_finite() { end } else { duration }));
    if let Some((start, end)) = ends {
        for (which, at) in [(0, start), (1, end)] {
            let grab = Rect::from_center_size(
                Pos2::new(x_of(at), frame.center().y),
                Vec2::new(MARGIN * 1.6, frame.height()),
            );
            let flag = ui.interact(grab, bar.id.with(("flag", which)), Sense::drag());
            if flag.hovered() || flag.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if flag.dragged() {
                flag_dragged = true;
                if let Some(pointer) = flag.interact_pointer_pos() {
                    let t = snapped(t_of(pointer.x));
                    *section = Some(if which == 0 {
                        (t.min(end - MIN_SECTION).max(first), section.map_or(end, |s| s.1))
                    } else {
                        (start, t.max(start + MIN_SECTION).min(duration))
                    });
                }
            }
        }
    }

    if !flag_dragged {
        if bar.dragged() || bar.clicked() {
            if let Some(pointer) = bar.interact_pointer_pos() {
                if ui.input(|input| input.modifiers.shift) && bar.dragged() {
                    let moved = f64::from(bar.drag_delta().x / track.width()) * span * FINE;
                    outcome.seek = Some((position + moved).clamp(first, duration));
                } else {
                    outcome.seek = Some(t_of(pointer.x));
                }
            }
        }
        if bar.hovered() {
            // Scrolling down goes forward; a notch of the wheel is about a second.
            let scrolled = ui.input(|input| input.smooth_scroll_delta.y);
            if scrolled != 0.0 {
                let notch = if ui.input(|input| input.modifiers.shift) { 500.0 } else { 50.0 };
                outcome.seek = Some((position - f64::from(scrolled) / notch).clamp(first, duration));
            }
        }
    }

    let hovered = bar.hover_pos().map(|pointer| t_of(pointer.x));
    if bar.secondary_clicked() {
        ui.memory_mut(|memory| memory.data.insert_temp(bar.id, hovered.unwrap_or(position)));
    }
    bar.context_menu(|ui| {
        let at = ui.memory(|memory| memory.data.get_temp::<f64>(bar.id)).unwrap_or(position);
        if ui.button("Practise from here  (A)").clicked() {
            *section = match *section {
                Some((_, end)) if end > at => Some((at, end)),
                _ => Some((at, f64::INFINITY)),
            };
            ui.close();
        }
        if ui.button("Practise up to here  (B)").clicked() {
            let start = section.map_or(first, |(start, _)| start);
            *section = (at > start).then_some((start, at));
            ui.close();
        }
        if ui.add_enabled(section.is_some(), egui::Button::new("Play the whole piece")).clicked() {
            *section = None;
            ui.close();
        }
    });

    paint(ui, frame, track, position, bars, ends, hovered, &x_of);

    if let Some(at) = hovered {
        let bar_text = bar_at(bars, at).map_or(String::new(), |n| format!("   bar {n}"));
        let shown = format!("{}{bar_text}", super::ui::clock_precise(at));
        bar.on_hover_ui_at_pointer(|ui| {
            ui.label(shown);
            ui.weak("click or drag to go there; hold Shift for fine control; right-click for a section");
        });
    }
    outcome
}

#[allow(clippy::too_many_arguments)]
fn paint(
    ui: &egui::Ui,
    frame: Rect,
    track: Rect,
    position: f64,
    bars: &[f64],
    ends: Option<(f64, f64)>,
    hovered: Option<f64>,
    x_of: &dyn Fn(f64) -> f32,
) {
    let painter = ui.painter_at(frame);
    let visuals = ui.visuals();
    let accent = crate::theme::GLOVER;
    let text = visuals.text_color();
    let weak = visuals.weak_text_color();

    painter.rect_filled(track, 6.0, Color32::from_rgb(0x1A, 0x1C, 0x24));

    let played = Rect::from_min_max(track.min, Pos2::new(x_of(position), track.bottom()));
    painter.rect_filled(played, 6.0, Color32::from_rgba_unmultiplied(0xEF, 0xEB, 0xDD, 56));

    if let Some((start, end)) = ends {
        let region =
            Rect::from_min_max(Pos2::new(x_of(start), track.top()), Pos2::new(x_of(end), track.bottom()));
        painter.rect_filled(region, 4.0, accent.gamma_multiply(0.30));
        for y in [region.top(), region.bottom()] {
            painter.line_segment(
                [Pos2::new(region.left(), y), Pos2::new(region.right(), y)],
                Stroke::new(2.0, accent),
            );
        }
    }

    let every = stride(bars.len(), track.width(), 7.0);
    let numbered = stride(bars.len(), track.width(), 44.0);
    for (i, start) in bars.iter().enumerate() {
        let x = x_of(*start);
        if i % every != 0 {
            continue;
        }
        let long = i % numbered == 0;
        let top = if long { track.top() } else { track.bottom() - track.height() * 0.4 };
        painter.line_segment(
            [Pos2::new(x, top), Pos2::new(x, track.bottom())],
            Stroke::new(1.0, weak.gamma_multiply(0.5)),
        );
        if long {
            painter.text(
                Pos2::new(x + 2.0, frame.top() + 1.0),
                Align2::LEFT_TOP,
                format!("{}", i + 1),
                FontId::proportional(11.0),
                weak,
            );
        }
    }

    if let Some(at) = hovered {
        let x = x_of(at);
        painter
            .line_segment([Pos2::new(x, track.top()), Pos2::new(x, track.bottom())], Stroke::new(1.0, weak));
    }

    if let Some((start, end)) = ends {
        for (letter, at) in [("A", start), ("B", end)] {
            let x = x_of(at);
            painter.line_segment(
                [Pos2::new(x, frame.top() + 11.0), Pos2::new(x, track.bottom() + 3.0)],
                Stroke::new(2.0, accent),
            );
            let tag = Rect::from_center_size(Pos2::new(x, frame.top() + 8.0), Vec2::new(18.0, 16.0));
            painter.rect_filled(tag, 4.0, accent);
            painter.text(
                tag.center(),
                Align2::CENTER_CENTER,
                letter,
                FontId::new(12.0, crate::theme::display()),
                crate::theme::LACQUER,
            );
        }
    }

    let x = x_of(position);
    painter.line_segment(
        [Pos2::new(x, track.top() - 3.0), Pos2::new(x, track.bottom() + 2.0)],
        Stroke::new(2.5, text),
    );
    painter.circle_filled(Pos2::new(x, track.center().y), 7.0, text);
    painter.circle_filled(Pos2::new(x, track.center().y), 2.5, Color32::from_rgb(0x0D, 0x0E, 0x12));
}

#[cfg(test)]
mod tests {
    use super::{bar_at, stride};

    #[test]
    fn bar_numbers_count_from_one_and_follow_the_music() {
        let bars = [0.0, 2.0, 4.0];
        assert_eq!(bar_at(&bars, -0.5), None);
        assert_eq!(bar_at(&bars, 0.0), Some(1));
        assert_eq!(bar_at(&bars, 3.9), Some(2));
        assert_eq!(bar_at(&bars, 4.0), Some(3));
        assert_eq!(bar_at(&[], 1.0), None);
    }

    #[test]
    fn ticks_thin_out_when_the_bars_would_crowd() {
        assert_eq!(stride(100, 1000.0, 7.0), 1, "ten pixels a bar is room enough");
        assert_eq!(stride(400, 1000.0, 7.0), 3, "2.5 pixels a bar leaves every third");
        assert_eq!(stride(0, 1000.0, 7.0), 1);
    }
}
