//! نظام التصميم: "مكتب مرتّب" — أداة هادئة ودقيقة، لون واحد للفعل الأساسي.
//! الألوان من أيقونة التطبيق (فيروزي وكهرماني)، بسمتين فاتحة وداكنة، وكل قيمة في مكان واحد.
//! الخط Segoe UI (موجود في ويندوز، فيعمل التطبيق بدون إنترنت) مع Tahoma للعربية.

use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{self, Color32, FontFamily, FontId, RichText, Rounding, Stroke, TextStyle};

use crate::icons::{self, Icon};

/// ألوان سمة واحدة. نسب التباين للنص ≥ 4.5:1 على خلفيته
pub struct Palette {
    pub bg: Color32,
    pub sidebar: Color32,
    pub surface: Color32,
    /// خلفية داخل البطاقة (منطقة الإفلات، مربعات الأرقام)
    pub inset: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    /// لون التطبيق للأيقونات وشريط التقدم والتمييز
    pub accent: Color32,
    /// تعبئة الزر الأساسي (أغمق قليلاً ليبقى النص الأبيض واضحاً)
    pub primary: Color32,
    pub primary_hover: Color32,
    /// خلفية العنصر المختار في القائمة الجانبية
    pub selected: Color32,
    pub amber: Color32,
    pub success: Color32,
    pub danger: Color32,
    pub chip: Color32,
    pub stripe: Color32,
    pub hover: Color32,
    pub toggle_off: Color32,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub const DARK: Palette = Palette {
    bg: hex(0x0F141C),
    sidebar: hex(0x131A24),
    surface: hex(0x1A222E),
    inset: hex(0x141B25),
    border: hex(0x283444),
    text: hex(0xE2E8F0),
    muted: hex(0x8C9AAC),
    accent: hex(0x2BA0A0),
    primary: hex(0x1A7074),
    primary_hover: hex(0x17676B),
    selected: hex(0x1D3B44),
    amber: hex(0xF0B24A),
    success: hex(0x56BE78),
    danger: hex(0xE86056),
    chip: hex(0x222C3A),
    stripe: hex(0x1E2734),
    hover: hex(0x243040),
    toggle_off: hex(0x404C5E),
};

pub const LIGHT: Palette = Palette {
    bg: hex(0xF3F5F8),
    sidebar: hex(0xE9EDF2),
    surface: hex(0xFFFFFF),
    inset: hex(0xF6F8FA),
    border: hex(0xDCE2E9),
    text: hex(0x17202B),
    muted: hex(0x5B6776),
    accent: hex(0x1D7A7A),
    primary: hex(0x1D7A7A),
    primary_hover: hex(0x166464),
    selected: hex(0xD5ECEC),
    amber: hex(0x9A6110),
    success: hex(0x26784A),
    danger: hex(0xC2453B),
    chip: hex(0xEDF1F5),
    stripe: hex(0xF6F8FA),
    hover: hex(0xE6EBF0),
    toggle_off: hex(0xC3CCD6),
};

static IS_DARK: AtomicBool = AtomicBool::new(true);

/// ألوان السمة الحالية
pub fn pal() -> &'static Palette {
    if IS_DARK.load(Ordering::Relaxed) { &DARK } else { &LIGHT }
}

pub const RADIUS: f32 = 10.0;
const SEMIBOLD: &str = "semibold";

/// الخطوط: Segoe UI للنص، Segoe UI Semibold للعناوين، Tahoma للحروف العربية،
/// وخطوط egui الافتراضية احتياطاً للرموز
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut load = |name: &str, file: &str| -> bool {
        match std::fs::read(format!("C:\\Windows\\Fonts\\{file}")) {
            Ok(bytes) => {
                fonts.font_data.insert(name.to_owned(), egui::FontData::from_owned(bytes));
                true
            }
            Err(_) => false,
        }
    };
    let regular = load("segoe", "segoeui.ttf");
    let semibold = load("segoe-semibold", "seguisb.ttf");
    let arabic = load("arabic", "tahoma.ttf");
    let arabic_bold = load("arabic-bold", "tahomabd.ttf");

    let fallback: Vec<String> = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let family = |main: Option<&str>, arabic: Option<&str>| -> Vec<String> {
        main.into_iter().chain(arabic).map(str::to_owned).chain(fallback.iter().cloned()).collect()
    };
    fonts.families.insert(
        FontFamily::Proportional,
        family(regular.then_some("segoe"), arabic.then_some("arabic")),
    );
    fonts.families.insert(
        FontFamily::Name(SEMIBOLD.into()),
        family(
            semibold.then_some("segoe-semibold").or(regular.then_some("segoe")),
            arabic_bold.then_some("arabic-bold").or(arabic.then_some("arabic")),
        ),
    );
    if arabic {
        fonts.families.entry(FontFamily::Monospace).or_default().push("arabic".to_owned());
    }
    ctx.set_fonts(fonts);
}

/// تطبيق السمة (تُستدعى عند البدء وعند تغيير الفاتح/الداكن)
pub fn apply(ctx: &egui::Context, dark: bool) {
    IS_DARK.store(dark, Ordering::Relaxed);
    let p = pal();
    let mut visuals = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    visuals.panel_fill = p.bg;
    visuals.window_fill = p.surface;
    visuals.extreme_bg_color = p.inset;
    visuals.faint_bg_color = p.stripe;
    visuals.override_text_color = Some(p.text);
    visuals.selection.bg_fill = p.primary;
    visuals.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    visuals.hyperlink_color = p.accent;
    visuals.window_rounding = Rounding::same(14.0);
    visuals.window_stroke = Stroke::new(1.0_f32, p.border);
    visuals.window_shadow = egui::epaint::Shadow { offset: egui::vec2(0.0, 8.0), blur: 28.0, spread: 0.0, color: Color32::from_black_alpha(if dark { 120 } else { 40 }) };
    visuals.popup_shadow = visuals.window_shadow;
    for w in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        w.rounding = Rounding::same(8.0);
    }
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border);
    visuals.widgets.inactive.weak_bg_fill = p.chip;
    visuals.widgets.inactive.bg_fill = p.chip;
    visuals.widgets.hovered.weak_bg_fill = p.hover;
    visuals.widgets.hovered.bg_fill = p.hover;
    visuals.widgets.active.weak_bg_fill = p.selected;
    visuals.widgets.active.bg_fill = p.selected;
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (TextStyle::Heading, FontId::new(26.0, FontFamily::Name(SEMIBOLD.into()))),
        (TextStyle::Body, FontId::proportional(15.0)),
        (TextStyle::Button, FontId::proportional(15.0)),
        (TextStyle::Small, FontId::proportional(13.0)),
        (TextStyle::Monospace, FontId::monospace(14.0)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 7.0);
    style.spacing.interact_size.y = 30.0;
    ctx.set_style(style);
}

/// نص عنوان بخط Semibold
pub fn strong(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).family(FontFamily::Name(SEMIBOLD.into())).size(size)
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).color(pal().muted)
}

/// بطاقة: سطح بحدود رفيعة وزوايا مستديرة
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let p = pal();
    egui::Frame::none()
        .fill(p.surface)
        .stroke(Stroke::new(1.0_f32, p.border))
        .rounding(12.0)
        .inner_margin(egui::Margin::same(20.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// عنوان خطوة: رقم في دائرة ثم نص
pub fn step_title(ui: &mut egui::Ui, number: &str, title: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 13.0, pal().primary);
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, number, FontId::new(14.0, FontFamily::Name(SEMIBOLD.into())), Color32::WHITE);
        ui.label(strong(title, 17.0));
    });
}

/// عنوان قسم بأيقونة
pub fn section_title(ui: &mut egui::Ui, icon: Icon, title: &str) {
    ui.horizontal(|ui| {
        icons::show(ui, icon, 20.0, pal().accent);
        ui.label(strong(title, 17.0));
    });
}

/// نص بأيقونة صغيرة قبله
pub fn icon_text(ui: &mut egui::Ui, icon: Icon, text: RichText, color: Color32) -> egui::Response {
    ui.horizontal(|ui| {
        icons::show(ui, icon, 16.0, color);
        // النص الطويل (مثل المسارات) ينتقل لسطر جديد بدل تجاوز حافة البطاقة
        ui.add(egui::Label::new(text).wrap(true));
    })
    .response
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Primary,
    Secondary,
    Ghost,
}

/// زر بأيقونة: حالات واضحة للمرور والضغط والتركيز بلوحة المفاتيح والتعطيل
pub fn button(ui: &mut egui::Ui, kind: Kind, icon: Option<Icon>, text: &str, enabled: bool) -> egui::Response {
    let p = pal();
    let font = FontId::new(15.0, if kind == Kind::Primary { FontFamily::Name(SEMIBOLD.into()) } else { FontFamily::Proportional });
    let probe = ui.painter().layout_no_wrap(text.to_owned(), font.clone(), p.text);
    let icon_space = if icon.is_some() { 26.0 } else { 0.0 };
    let height = if kind == Kind::Primary { 42.0 } else { 36.0 };
    let width = (probe.size().x + icon_space + 32.0).max(if kind == Kind::Primary { 180.0 } else { 0.0 });
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    // اسم الزر لقارئ الشاشة وأدوات ويندوز
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, text));

    let hovered = enabled && response.hovered();
    let pressed = enabled && response.is_pointer_button_down_on();
    let (fill, fg, border) = match kind {
        Kind::Primary if !enabled => (p.toggle_off, p.muted, Color32::TRANSPARENT),
        Kind::Primary => (if hovered || pressed { p.primary_hover } else { p.primary }, Color32::WHITE, Color32::TRANSPARENT),
        Kind::Secondary => (if pressed { p.selected } else if hovered { p.hover } else { p.chip }, if enabled { p.text } else { p.muted }, p.border),
        Kind::Ghost => (if pressed { p.selected } else if hovered { p.hover } else { Color32::TRANSPARENT }, if enabled { p.text } else { p.muted }, Color32::TRANSPARENT),
    };
    let rect_drawn = if pressed { rect.shrink(1.0) } else { rect };
    ui.painter().rect(rect_drawn, 8.0, fill, Stroke::new(1.0_f32, border));
    if response.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), 10.0, Stroke::new(2.0_f32, p.accent));
    }

    let content_width = probe.size().x + icon_space;
    let mut x = rect.center().x - content_width / 2.0;
    if let Some(icon) = icon {
        let icon_rect = egui::Rect::from_center_size(egui::pos2(x + 9.0, rect.center().y), egui::vec2(18.0, 18.0));
        icons::paint(ui.painter(), icon_rect, icon, fg, 0.0);
        x += icon_space;
    }
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, fg);
    ui.painter().galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, fg);

    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// عنصر في القائمة الجانبية
pub fn nav_item(ui: &mut egui::Ui, icon: Icon, text: &str, selected: bool, enabled: bool) -> egui::Response {
    let p = pal();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 42.0), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    // زر (يدعم أمر "الضغط" في أدوات ويندوز وقارئ الشاشة)، والحالة المختارة ظاهرة بصرياً
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, text));
    let hovered = enabled && response.hovered();
    if selected {
        ui.painter().rect_filled(rect, 8.0, p.selected);
        // شريط صغير بلون التطبيق يدل على الصفحة الحالية
        let bar = egui::Rect::from_min_size(rect.left_center() - egui::vec2(0.0, 10.0), egui::vec2(3.0, 20.0));
        ui.painter().rect_filled(bar, 2.0, p.accent);
    } else if hovered {
        ui.painter().rect_filled(rect, 8.0, p.hover);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(rect, 8.0, Stroke::new(2.0_f32, p.accent));
    }
    let color = if !enabled {
        p.muted.gamma_multiply(0.7)
    } else if selected {
        p.accent
    } else {
        p.muted
    };
    let icon_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + 26.0, rect.center().y), egui::vec2(20.0, 20.0));
    icons::paint(ui.painter(), icon_rect, icon, color, 0.0);
    let text_color = if !enabled { p.muted.gamma_multiply(0.7) } else if selected { p.text } else { p.muted };
    let font = FontId::new(15.5, if selected { FontFamily::Name(SEMIBOLD.into()) } else { FontFamily::Proportional });
    ui.painter().text(egui::pos2(rect.left() + 48.0, rect.center().y), egui::Align2::LEFT_CENTER, text, font, text_color);
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// مفتاح تشغيل/إيقاف مع عنوان وسطر شرح
pub fn toggle_row(ui: &mut egui::Ui, enabled: bool, on: &mut bool, title: &str, help: &str) -> bool {
    let p = pal();
    let mut changed = false;
    ui.horizontal(|ui| {
        let size = egui::vec2(44.0, 24.0);
        let (rect, response) = ui.allocate_exact_size(size, if enabled { egui::Sense::click() } else { egui::Sense::hover() });
        if response.clicked() {
            *on = !*on;
            changed = true;
        }
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, *on, title));
        let t = ui.ctx().animate_bool(response.id, *on);
        let fill = if !enabled { p.toggle_off.gamma_multiply(0.6) } else { lerp_color(p.toggle_off, p.primary, t) };
        ui.painter().rect_filled(rect, 12.0, fill);
        if response.has_focus() {
            ui.painter().rect_stroke(rect.expand(2.0), 14.0, Stroke::new(2.0_f32, p.accent));
        }
        let x = egui::lerp(rect.left() + 12.0..=rect.right() - 12.0, t);
        ui.painter().circle_filled(egui::pos2(x, rect.center().y), 9.0, if enabled { Color32::WHITE } else { p.muted });
        if enabled {
            response.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        ui.vertical(|ui| {
            ui.label(strong(title, 15.0).color(if enabled { p.text } else { p.muted }));
            ui.label(RichText::new(help).small().color(p.muted));
        });
    });
    changed
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

/// مربع إحصاء: أيقونة ورقم كبير ووصف
pub fn stat(ui: &mut egui::Ui, width: f32, icon: Icon, value: usize, label: &str, color: Color32) {
    let p = pal();
    egui::Frame::none()
        .fill(p.surface)
        .stroke(Stroke::new(1.0_f32, p.border))
        .rounding(RADIUS)
        .inner_margin(egui::Margin::symmetric(14.0, 12.0))
        .show(ui, |ui| {
            // داخل صف أفقي: المحتوى عمودي والعرض ثابت حتى لا يتجاوز الصف حافة النافذة
            ui.set_width(width);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    icons::show(ui, icon, 18.0, color);
                    ui.label(strong(value.to_string(), 24.0).color(color));
                });
                ui.add(egui::Label::new(RichText::new(label).small().color(p.muted)).truncate(true));
            });
        });
}

/// شارة صغيرة بأيقونة. حجمها يُحسب قبل وضعها حتى ينتقل الصف لسطر جديد عند امتلائه
pub fn chip(ui: &mut egui::Ui, icon: Icon, text: &str) {
    let p = pal();
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(13.0), p.text);
    let size = egui::vec2(galley.size().x + 44.0, 30.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect_filled(rect, 15.0, p.chip);
    let icon_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + 19.0, rect.center().y), egui::vec2(15.0, 15.0));
    icons::paint(ui.painter(), icon_rect, icon, p.accent, 0.0);
    ui.painter().galley(egui::pos2(rect.left() + 32.0, rect.center().y - galley.size().y / 2.0), galley, p.text);
}

/// اختيار من عدة خيارات متجاورة (مثل: System / Light / Dark)
pub fn segmented(ui: &mut egui::Ui, options: &[&str], selected: &mut usize) -> bool {
    let p = pal();
    let mut changed = false;
    egui::Frame::none().fill(p.chip).rounding(9.0).inner_margin(egui::Margin::same(3.0)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (i, option) in options.iter().enumerate() {
                let active = *selected == i;
                let galley = ui.painter().layout_no_wrap(option.to_string(), FontId::proportional(14.0), p.text);
                let (rect, response) = ui.allocate_exact_size(egui::vec2(galley.size().x + 28.0, 30.0), egui::Sense::click());
                response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, active, *option));
                if active {
                    ui.painter().rect(rect, 7.0, p.surface, Stroke::new(1.0_f32, p.border));
                } else if response.hovered() {
                    ui.painter().rect_filled(rect, 7.0, p.hover);
                }
                let color = if active { p.text } else { p.muted };
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, *option, FontId::proportional(14.0), color);
                if response.clicked() && !active {
                    *selected = i;
                    changed = true;
                }
                response.on_hover_cursor(egui::CursorIcon::PointingHand);
            }
        });
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(c: Color32) -> f64 {
        let channel = |v: u8| {
            let s = v as f64 / 255.0;
            if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// نص الواجهة يحقق 4.5:1 على خلفياته في السمتين
    #[test]
    fn text_contrast_meets_wcag_aa() {
        for p in [&DARK, &LIGHT] {
            for bg in [p.bg, p.surface, p.sidebar, p.inset, p.chip] {
                assert!(contrast(p.text, bg) >= 4.5);
                assert!(contrast(p.muted, bg) >= 4.5, "muted on {bg:?}: {:.2}", contrast(p.muted, bg));
            }
            for c in [p.accent, p.amber, p.success, p.danger] {
                assert!(contrast(c, p.surface) >= 4.5, "{c:?} on surface: {:.2}", contrast(c, p.surface));
            }
            assert!(contrast(Color32::WHITE, p.primary) >= 4.5, "white on primary: {:.2}", contrast(Color32::WHITE, p.primary));
            assert!(contrast(Color32::WHITE, p.primary_hover) >= 4.5);
        }
    }
}
