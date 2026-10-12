//! مظهر التطبيق: ألوان من الأيقونة (فيروزي وكهرماني على خلفية كحلية داكنة)، وزوايا مستديرة،
//! وخطوط أكبر، ومفتاح تشغيل/إيقاف، وبطاقات وإحصاءات.

use eframe::egui::{self, Color32, FontId, RichText, Rounding, Stroke, TextStyle};

pub const ACCENT: Color32 = Color32::from_rgb(38, 150, 150);
pub const ACCENT_DARK: Color32 = Color32::from_rgb(26, 112, 116);
pub const AMBER: Color32 = Color32::from_rgb(240, 178, 74);
pub const GREEN: Color32 = Color32::from_rgb(86, 190, 120);
pub const RED: Color32 = Color32::from_rgb(232, 96, 86);
pub const BG: Color32 = Color32::from_rgb(15, 20, 28);
pub const SIDEBAR: Color32 = Color32::from_rgb(19, 26, 36);
pub const CARD: Color32 = Color32::from_rgb(26, 34, 46);
pub const CARD_BORDER: Color32 = Color32::from_rgb(40, 52, 68);
pub const TEXT: Color32 = Color32::from_rgb(226, 232, 240);
pub const MUTED: Color32 = Color32::from_rgb(140, 154, 172);

pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = CARD;
    visuals.extreme_bg_color = Color32::from_rgb(12, 16, 22);
    visuals.faint_bg_color = Color32::from_rgb(22, 29, 40);
    visuals.override_text_color = Some(TEXT);
    visuals.selection.bg_fill = ACCENT_DARK;
    visuals.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    visuals.hyperlink_color = ACCENT;
    visuals.window_rounding = Rounding::same(12.0);
    visuals.window_stroke = Stroke::new(1.0_f32, CARD_BORDER);
    for w in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        w.rounding = Rounding::same(8.0);
    }
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(34, 44, 58);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(34, 44, 58);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(44, 58, 76);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(44, 58, 76);
    visuals.widgets.active.weak_bg_fill = ACCENT_DARK;
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (TextStyle::Heading, FontId::proportional(24.0)),
        (TextStyle::Body, FontId::proportional(15.0)),
        (TextStyle::Button, FontId::proportional(15.0)),
        (TextStyle::Small, FontId::proportional(12.5)),
        (TextStyle::Monospace, FontId::monospace(14.0)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 7.0);
    style.spacing.interact_size.y = 30.0;
    ctx.set_style(style);
}

/// بطاقة: خلفية أفتح قليلاً وحدود وزوايا مستديرة
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, CARD_BORDER))
        .rounding(12.0)
        .inner_margin(egui::Margin::same(18.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// عنوان خطوة داخل بطاقة: رقم في دائرة ثم نص
pub fn step_title(ui: &mut egui::Ui, number: &str, title: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 13.0, ACCENT);
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, number, FontId::proportional(14.0), Color32::WHITE);
        ui.label(RichText::new(title).size(17.0).strong());
    });
}

/// زر أساسي بلون التطبيق
pub fn primary_button(ui: &mut egui::Ui, enabled: bool, text: &str) -> egui::Response {
    let button = egui::Button::new(RichText::new(text).size(16.0).strong().color(Color32::WHITE))
        .fill(if enabled { ACCENT } else { Color32::from_rgb(52, 64, 80) })
        .min_size(egui::vec2(220.0, 42.0));
    ui.add_enabled(enabled, button)
}

/// مفتاح تشغيل/إيقاف مع عنوان وسطر شرح
pub fn toggle_row(ui: &mut egui::Ui, enabled: bool, on: &mut bool, title: &str, help: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let size = egui::vec2(44.0, 24.0);
        let (rect, response) = ui.allocate_exact_size(size, if enabled { egui::Sense::click() } else { egui::Sense::hover() });
        if response.clicked() {
            *on = !*on;
            changed = true;
        }
        let t = ui.ctx().animate_bool(response.id, *on);
        let fill = if !enabled {
            Color32::from_rgb(48, 58, 72)
        } else {
            lerp_color(Color32::from_rgb(64, 76, 94), ACCENT, t)
        };
        ui.painter().rect_filled(rect, 12.0, fill);
        let x = egui::lerp(rect.left() + 12.0..=rect.right() - 12.0, t);
        ui.painter().circle_filled(egui::pos2(x, rect.center().y), 9.0, if enabled { Color32::WHITE } else { MUTED });
        ui.vertical(|ui| {
            ui.label(RichText::new(title).strong().color(if enabled { TEXT } else { MUTED }));
            ui.label(RichText::new(help).small().color(MUTED));
        });
    });
    changed
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

/// مربع إحصاء: رقم كبير ووصف تحته
pub fn stat(ui: &mut egui::Ui, width: f32, value: usize, label: &str, color: Color32) {
    egui::Frame::none()
        .fill(Color32::from_rgb(20, 27, 37))
        .rounding(10.0)
        .inner_margin(egui::Margin::symmetric(14.0, 10.0))
        .show(ui, |ui| {
            // داخل صف أفقي: الرقم والوصف فوق بعض، والعرض ثابت حتى لا يتجاوز الصف حافة النافذة
            ui.set_width(width);
            ui.vertical(|ui| {
                ui.label(RichText::new(value.to_string()).size(26.0).strong().color(color));
                ui.add(egui::Label::new(RichText::new(label).small().color(MUTED)).truncate(true));
            });
        });
}
