//! How the viewer's interface looks: lacquer-black chrome, ivory text, Nunito for its round
//! letters, and glover yellow for the one thing to press.

use bevy_egui::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

/// The chrome around the stage.
pub(crate) const LACQUER: Color32 = Color32::from_rgb(0x0D, 0x0E, 0x12);
/// Cards and menus, a step above the lacquer.
pub(crate) const FELT: Color32 = Color32::from_rgb(0x17, 0x19, 0x20);
/// Lines between things.
pub(crate) const EDGE: Color32 = Color32::from_rgb(0x2B, 0x2E, 0x38);
/// Text, warm like a key's ivory rather than white.
pub(crate) const IVORY: Color32 = Color32::from_rgb(0xEF, 0xEB, 0xDD);
/// Less important text.
pub(crate) const DIM: Color32 = Color32::from_rgb(0x9B, 0x98, 0x8D);
/// Glover's yellow: the primary action, and where you are in the piece.
pub(crate) const GLOVER: Color32 = Color32::from_rgb(0xFF, 0xCD, 0x00);

const REGULAR: &[u8] = include_bytes!("../../../assets/fonts/Nunito-Regular.ttf");
const SEMIBOLD: &[u8] = include_bytes!("../../../assets/fonts/Nunito-SemiBold.ttf");
const EXTRABOLD: &[u8] = include_bytes!("../../../assets/fonts/Nunito-ExtraBold.ttf");
const GLOVER_PICTURE: &[u8] = include_bytes!("../../../assets/branding/glover.png");

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the fonts have been handed over and are now in use. egui takes them on at the
/// start of the next frame, and drawing with a family it does not have yet is a crash.
static FONTS_READY: AtomicBool = AtomicBool::new(false);

/// The family for titles.
pub(crate) fn display() -> FontFamily {
    named("display")
}

/// The family for buttons and labels that want a little more weight.
pub(crate) fn medium() -> FontFamily {
    named("medium")
}

fn named(name: &str) -> FontFamily {
    if FONTS_READY.load(Ordering::Acquire) {
        FontFamily::Name(name.into())
    } else {
        FontFamily::Proportional
    }
}

/// Sets the fonts, colours and spacing. Called every frame until it has finished: the first
/// call hands over the fonts, the second sets the styles that use them.
pub(crate) fn apply(ctx: &egui::Context) -> bool {
    if FONTS_READY.load(Ordering::Acquire) {
        return true;
    }
    let first = !ctx.data(|data| data.get_temp::<bool>(egui::Id::new("fonts-given"))).unwrap_or(false);
    if first {
        give_fonts(ctx);
        ctx.data_mut(|data| data.insert_temp(egui::Id::new("fonts-given"), true));
        return false;
    }
    FONTS_READY.store(true, Ordering::Release);
    set_style(ctx);
    true
}

fn give_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [("nunito", REGULAR), ("nunito-semibold", SEMIBOLD), ("nunito-extrabold", EXTRABOLD)]
    {
        fonts.font_data.insert(name.into(), egui::FontData::from_static(bytes).into());
    }
    // The built-in fonts stay behind Nunito, for the arrows and symbols it does not have.
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "nunito".into());
    let fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut titles = fallback.clone();
    titles.insert(0, "nunito-extrabold".into());
    fonts.families.insert(FontFamily::Name("display".into()), titles);
    let mut medium = fallback;
    medium.insert(0, "nunito-semibold".into());
    fonts.families.insert(FontFamily::Name("medium".into()), medium);
    ctx.set_fonts(fonts);
}

fn set_style(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(15.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(15.0, medium())),
            (TextStyle::Heading, FontId::new(28.0, display())),
            (TextStyle::Monospace, FontId::new(14.0, FontFamily::Monospace)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(14.0, 6.0);
        style.spacing.interact_size.y = 32.0;
        style.spacing.menu_margin = egui::Margin::symmetric(8, 6);
        style.spacing.window_margin = egui::Margin::same(24);

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.override_text_color = Some(IVORY);
        v.panel_fill = LACQUER;
        v.window_fill = FELT;
        v.window_stroke = Stroke::new(1.0, EDGE);
        v.window_corner_radius = CornerRadius::same(18);
        v.menu_corner_radius = CornerRadius::same(12);
        v.window_shadow =
            egui::Shadow { offset: [0, 12], blur: 40, spread: 0, color: Color32::from_black_alpha(160) };
        v.popup_shadow =
            egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(140) };
        v.extreme_bg_color = Color32::from_rgb(0x08, 0x09, 0x0C);
        v.faint_bg_color = Color32::from_rgb(0x12, 0x14, 0x1A);
        v.hyperlink_color = GLOVER;
        v.selection.bg_fill = Color32::from_rgb(0x5E, 0x4C, 0x00);
        v.selection.stroke = Stroke::new(1.0, GLOVER);

        let rounding = CornerRadius::same(10);
        v.widgets.noninteractive.bg_fill = FELT;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, EDGE);
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, IVORY);
        v.widgets.noninteractive.corner_radius = rounding;
        for (state, fill) in [
            (&mut v.widgets.inactive, Color32::from_rgb(0x20, 0x22, 0x2B)),
            (&mut v.widgets.hovered, Color32::from_rgb(0x2C, 0x2F, 0x3B)),
            (&mut v.widgets.active, Color32::from_rgb(0x38, 0x3C, 0x4A)),
            (&mut v.widgets.open, Color32::from_rgb(0x2C, 0x2F, 0x3B)),
        ] {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.bg_stroke = Stroke::NONE;
            state.fg_stroke = Stroke::new(1.0, IVORY);
            state.corner_radius = rounding;
            state.expansion = 0.0;
        }
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, EDGE);
    });
}

/// The picture of glover, loaded the first time it is asked for.
pub(crate) fn glover(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let id = egui::Id::new("glover-texture");
    if let Some(texture) = ctx.data(|data| data.get_temp::<egui::TextureHandle>(id)) {
        return Some(texture);
    }
    let image = image::load_from_memory(GLOVER_PICTURE).ok()?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let colour = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    let texture = ctx.load_texture("glover", colour, egui::TextureOptions::LINEAR);
    ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
    Some(texture)
}

/// Shows glover `width` wide.
pub(crate) fn show_glover(ui: &mut egui::Ui, width: f32) {
    if let Some(texture) = glover(ui.ctx()) {
        let size = texture.size_vec2();
        ui.image((texture.id(), egui::vec2(width, width * size.y / size.x)));
    }
}

/// The button for the thing to do next: yellow, with dark text, pill shaped.
pub(crate) fn primary(ui: &mut egui::Ui, text: &str, size: egui::Vec2) -> egui::Response {
    let label = egui::RichText::new(text).font(FontId::new(17.0, display())).color(LACQUER);
    ui.add(egui::Button::new(label).fill(GLOVER).corner_radius(CornerRadius::same(255)).min_size(size))
}

/// A quieter button, for a second choice.
pub(crate) fn quiet(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(text).corner_radius(CornerRadius::same(255)))
}

/// The round play and pause button, with its symbol drawn rather than typed so it looks the
/// same everywhere.
pub(crate) fn play_button(ui: &mut egui::Ui, playing: bool) -> egui::Response {
    let (area, response) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::click());
    let fill = if response.is_pointer_button_down_on() {
        Color32::from_rgb(0xE0, 0xB4, 0x00)
    } else if response.hovered() {
        Color32::from_rgb(0xFF, 0xD9, 0x33)
    } else {
        GLOVER
    };
    let painter = ui.painter();
    painter.circle_filled(area.center(), 22.0, fill);
    let c = area.center();
    if playing {
        for dx in [-5.5, 5.5] {
            let bar = egui::Rect::from_center_size(egui::pos2(c.x + dx, c.y), egui::vec2(5.0, 16.0));
            painter.rect_filled(bar, CornerRadius::same(2), LACQUER);
        }
    } else {
        // Nudged right: a triangle looks centred when its middle of mass is.
        let points = vec![
            egui::pos2(c.x - 5.0, c.y - 9.0),
            egui::pos2(c.x - 5.0, c.y + 9.0),
            egui::pos2(c.x + 9.0, c.y),
        ];
        painter.add(egui::Shape::convex_polygon(points, LACQUER, Stroke::NONE));
    }
    response.on_hover_text(if playing { "Pause  (Space)" } else { "Play  (Space)" })
}

/// A small on/off button, lit yellow while on.
pub(crate) fn toggle_pill(ui: &mut egui::Ui, text: &str, on: bool) -> egui::Response {
    let colour = if on { LACQUER } else { IVORY };
    let label = egui::RichText::new(text).font(FontId::new(15.0, display())).color(colour);
    let mut button =
        egui::Button::new(label).corner_radius(CornerRadius::same(255)).min_size(egui::vec2(38.0, 30.0));
    if on {
        button = button.fill(GLOVER);
    }
    ui.add(button)
}

/// The button that goes back to the start, its symbol drawn.
pub(crate) fn restart_button(ui: &mut egui::Ui) -> egui::Response {
    let (area, response) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::click());
    let colour = if response.hovered() { IVORY } else { DIM };
    let c = area.center();
    let painter = ui.painter();
    painter.rect_filled(
        egui::Rect::from_center_size(egui::pos2(c.x - 7.0, c.y), egui::vec2(3.0, 14.0)),
        CornerRadius::same(1),
        colour,
    );
    let points =
        vec![egui::pos2(c.x + 7.0, c.y - 7.0), egui::pos2(c.x + 7.0, c.y + 7.0), egui::pos2(c.x - 4.0, c.y)];
    painter.add(egui::Shape::convex_polygon(points, colour, Stroke::NONE));
    response.on_hover_text("Back to the start  (Home)")
}
