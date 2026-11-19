//! The look: Mimizan Lab's. A grey room, dark glass on it, grey words,
//! and amber for the one button that keeps something and for what is
//! being worked on.

use egui::{
    pos2, vec2, Color32, FontId, Galley, Response, RichText, Sense, Stroke, StrokeKind, TextStyle, Ui,
    Widget, WidgetInfo, WidgetText, WidgetType,
};
use std::sync::Arc;

/// The room the picture lies in.
pub const STAGE: Color32 = Color32::from_gray(28);
/// The toolbar.
pub const BAR_FILL: Color32 = Color32::from_gray(18);
pub const AMBER: Color32 = Color32::from_rgb(255, 170, 60);
/// Went well, and went wrong.
pub const GOOD: Color32 = Color32::from_rgb(134, 190, 138);
pub const BAD: Color32 = Color32::from_rgb(236, 118, 100);

/// Height of a row of controls, points.
pub const ROW_H: f32 = 26.0;
pub const TEXT: f32 = 13.0;
pub const SMALL: f32 = 11.0;
const GAP: f32 = 6.0;
const PAD: f32 = 9.0;
pub const RADIUS: f32 = 6.0;

pub const INK: Color32 = Color32::from_gray(210);
pub const INK_LIT: Color32 = Color32::from_gray(242);
pub const INK_QUIET: Color32 = Color32::from_gray(166);
pub const INK_SMALL: Color32 = Color32::from_gray(140);
pub const SHOWN: Color32 = Color32::from_gray(48);
pub const SHOWN_HOVER: Color32 = Color32::from_gray(58);
pub const HOVER: Color32 = Color32::from_gray(40);
const KEEP_HOVER: Color32 = Color32::from_rgb(255, 188, 98);
const KEEP_INK: Color32 = Color32::from_gray(27);
const KEEP_KEY: Color32 = Color32::from_rgb(90, 58, 12);
const BAR: Color32 = Color32::from_gray(60);

/// Always dark, with the sizes the controls here are drawn in.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = STAGE;
    visuals.selection.bg_fill = Color32::from_rgb(112, 76, 28);
    visuals.selection.stroke = Stroke::new(1.0, AMBER);
    ctx.set_visuals(visuals);
    ctx.all_styles_mut(|style| {
        for (text, size) in [
            (TextStyle::Body, TEXT),
            (TextStyle::Button, TEXT),
            (TextStyle::Small, SMALL),
            (TextStyle::Monospace, 12.0),
            (TextStyle::Heading, 16.0),
        ] {
            style.text_styles.insert(text, FontId::proportional(size));
        }
    });
}

/// Dark glass with a soft shadow: what every panel is made of.
pub fn glass() -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(14, 14, 16, 222))
        .stroke(Stroke::new(1.0, Color32::from_white_alpha(20)))
        .corner_radius(10.0)
        .inner_margin(12.0)
        .shadow(egui::Shadow { offset: [0, 6], blur: 26, spread: 0, color: Color32::from_black_alpha(150) })
}

/// A panel of glass as wide as the room it is in, under its name. What
/// stands right of the name is `beside`.
pub fn panel(ui: &mut Ui, title: &str, beside: impl FnOnce(&mut Ui), add: impl FnOnce(&mut Ui)) {
    glass().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).strong().color(INK_LIT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), beside);
        });
        ui.separator();
        add(ui);
    });
}

/// Width of the names of the values, points: the longest of them fits.
const KEY_W: f32 = 66.0;

/// The name of a value, and the value: a long one is cut and never makes
/// the panel wider.
pub fn row(ui: &mut Ui, key: &str, value: impl Into<WidgetText>) -> Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (rect, _) = ui.allocate_exact_size(vec2(KEY_W, 17.0), Sense::hover());
        let at = rect.left_center();
        ui.painter().text(at, egui::Align2::LEFT_CENTER, key, FontId::proportional(TEXT), INK_SMALL);
        ui.add(egui::Label::new(value).truncate())
    })
    .inner
}

/// Rows of [`row`], close to one another.
pub fn rows(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        add(ui);
    });
}

pub fn value(text: impl Into<String>) -> RichText {
    RichText::new(text).color(INK)
}

pub fn quiet(text: impl Into<String>) -> RichText {
    RichText::new(text).color(INK_SMALL)
}

#[derive(Clone, Copy)]
enum Look {
    /// One of several, or shown or not; lit while it is the one.
    Toggle(bool),
    /// Done once, when pressed.
    Action,
    /// The step that keeps something: filled.
    Keep,
}

/// A button. Each kind has one look, so what it does is read before it is
/// pressed.
pub struct Chip<'a> {
    look: Look,
    label: &'a str,
    key: Option<&'a str>,
    height: f32,
    least: f32,
}

impl<'a> Chip<'a> {
    fn new(look: Look, label: &'a str) -> Self {
        Self { look, label, key: None, height: ROW_H, least: 0.0 }
    }

    pub fn toggle(label: &'a str, lit: bool) -> Self {
        Self::new(Look::Toggle(lit), label)
    }

    pub fn action(label: &'a str) -> Self {
        Self::new(Look::Action, label)
    }

    pub fn keep(label: &'a str) -> Self {
        Self::new(Look::Keep, label)
    }

    /// The key that does the same, small, after the word.
    pub fn key(mut self, key: &'a str) -> Self {
        self.key = Some(key);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// At least this wide: a button that fills the row it stands in.
    pub fn wide(mut self, width: f32) -> Self {
        self.least = width;
        self
    }
}

pub fn galley(ui: &Ui, text: &str, size: f32) -> Arc<Galley> {
    ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(size), Color32::PLACEHOLDER)
}

/// One line of `text`, cut with an ellipsis at `width`.
pub fn elided(ui: &Ui, text: &str, size: f32, colour: Color32, width: f32) -> Arc<Galley> {
    let mut job =
        egui::text::LayoutJob::simple_singleline(text.to_owned(), FontId::proportional(size), colour);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(0.0));
    ui.painter().layout_job(job)
}

impl Widget for Chip<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let label = galley(ui, self.label, TEXT);
        let key = self.key.map(|k| galley(ui, k, SMALL));
        let width = 2.0 * PAD + label.size().x + key.as_ref().map_or(0.0, |k| GAP + k.size().x);
        let (rect, response) =
            ui.allocate_exact_size(vec2(width.max(self.least), self.height), Sense::click());
        let enabled = ui.is_enabled();
        response.widget_info(|| match self.look {
            Look::Toggle(lit) => WidgetInfo::selected(WidgetType::Button, enabled, lit, self.label),
            Look::Action | Look::Keep => WidgetInfo::labeled(WidgetType::Button, enabled, self.label),
        });
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let hot = enabled && response.hovered();
        let (fill, ink, small) = match self.look {
            Look::Toggle(true) => (Some(if hot { SHOWN_HOVER } else { SHOWN }), INK_LIT, INK_SMALL),
            Look::Toggle(false) => (hot.then_some(HOVER), if hot { INK } else { INK_QUIET }, INK_SMALL),
            Look::Action => (hot.then_some(HOVER), INK, INK_SMALL),
            Look::Keep => (Some(if hot { KEEP_HOVER } else { AMBER }), KEEP_INK, KEEP_KEY),
        };
        let painter = ui.painter();
        if let Some(fill) = fill {
            painter.rect_filled(rect, RADIUS, fill);
        }
        let mid = rect.center().y;
        let mut x = rect.left() + PAD;
        for (text, colour) in [(Some(label), ink), (key, small)] {
            if let Some(text) = text {
                let w = text.size().x;
                painter.galley(pos2(x, mid - text.size().y / 2.0), text, colour);
                x += w + GAP;
            }
        }
        response
    }
}

/// The bar between two groups of a row.
pub fn sep(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(13.0, ROW_H), Sense::hover());
    let c = rect.center();
    ui.painter().line_segment([pos2(c.x, c.y - 8.0), pos2(c.x, c.y + 8.0)], Stroke::new(1.0, BAR));
}

/// The name of the group that follows.
pub fn caption(ui: &mut Ui, text: &str) {
    ui.label(quiet(text));
}

/// How far something is: a thin amber bar on a grey one.
pub fn progress(ui: &mut Ui, share: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, SHOWN);
    let mut done = rect;
    done.set_width(rect.width() * share.clamp(0.0, 1.0));
    painter.rect_filled(done, 2.0, AMBER);
}

/// The box a file is marked in.
pub fn paint_mark(ui: &Ui, rect: egui::Rect, marked: bool, hot: bool) {
    let painter = ui.painter();
    if marked {
        painter.rect_filled(rect, 3.0, if hot { KEEP_HOVER } else { AMBER });
        let p = |x: f32, y: f32| pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height());
        let tick = vec![p(0.24, 0.52), p(0.43, 0.71), p(0.77, 0.30)];
        painter.add(egui::Shape::line(tick, Stroke::new(1.6, KEEP_INK)));
    } else {
        let edge = if hot { INK_QUIET } else { Color32::from_gray(96) };
        painter.rect_stroke(rect, 3.0, Stroke::new(1.0, edge), StrokeKind::Inside);
    }
}

#[cfg(test)]
mod tests {
    use super::Chip;
    use egui::{pos2, Event, Modifiers, PointerButton, Rect};

    fn clicked(enabled: bool) -> bool {
        let ctx = egui::Context::default();
        let at = pos2(24.0, 16.0);
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mut time = 0.0;
        let mut clicked = false;
        for events in [vec![Event::PointerMoved(at)], vec![button(true)], vec![button(false)]] {
            time += 0.02;
            let screen_rect = Some(Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(400.0, 300.0)));
            let input = egui::RawInput { events, time: Some(time), screen_rect, ..Default::default() };
            let _ = ctx.run_ui(input, |ui| {
                let chip = ui.add_enabled(enabled, Chip::keep("Convert 3 files"));
                assert!(chip.rect.contains(at), "the press is not on the chip: {:?}", chip.rect);
                clicked |= chip.clicked();
            });
        }
        clicked
    }

    #[test]
    fn a_chip_is_pressed_like_a_button_and_not_while_it_is_disabled() {
        assert!(clicked(true));
        assert!(!clicked(false));
    }

    #[test]
    fn a_chip_is_as_wide_as_it_is_told_and_no_narrower_than_its_words() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let wide = ui.add(Chip::keep("Convert").wide(280.0).height(30.0)).rect;
            assert!((wide.width() - 280.0).abs() < 1.0 && (wide.height() - 30.0).abs() < 1.0);
            let bare = ui.add(Chip::action("All")).rect.width();
            let keyed = ui.add(Chip::action("All").key("Ctrl+A").wide(4.0)).rect.width();
            assert!(keyed > bare && bare > 4.0);
        });
    }
}
