//! مجموعة أيقونات Array: خطية على شبكة 24×24، سماكة 1.75، أطراف وزوايا مستديرة.
//! كل أيقونة معرّفة مرة واحدة كأشكال هندسية: تُرسم في الواجهة بلون السمة الحالية،
//! وتُصدَّر من نفس التعريف ملفات SVG في assets/icons.

use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke};

pub const STROKE: f32 = 1.75;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Organize,
    History,
    Settings,
    About,
    Folder,
    Search,
    Check,
    Close,
    Undo,
    Shield,
    Person,
    Lock,
    Edit,
    Trash,
    Drop,
    Music,
    Photo,
    Video,
    Document,
    Archive,
    ArrowRight,
    Warning,
    External,
    Clock,
    Spinner,
}

#[cfg(test)]
pub const ALL: [Icon; 25] = [
    Icon::Organize,
    Icon::History,
    Icon::Settings,
    Icon::About,
    Icon::Folder,
    Icon::Search,
    Icon::Check,
    Icon::Close,
    Icon::Undo,
    Icon::Shield,
    Icon::Person,
    Icon::Lock,
    Icon::Edit,
    Icon::Trash,
    Icon::Drop,
    Icon::Music,
    Icon::Photo,
    Icon::Video,
    Icon::Document,
    Icon::Archive,
    Icon::ArrowRight,
    Icon::Warning,
    Icon::External,
    Icon::Clock,
    Icon::Spinner,
];

/// عناصر الرسم على الشبكة 24×24
enum Part {
    /// خط مفتوح
    Line(&'static [(f32, f32)]),
    /// شكل مغلق
    Closed(&'static [(f32, f32)]),
    Circle(f32, f32, f32),
    /// قوس: المركز، نصف القطر، زاوية البداية والنهاية بالدرجات (0 = يمين، تزداد مع عقارب الساعة)
    Arc(f32, f32, f32, f32, f32),
    /// مستطيل بزوايا مستديرة: x0, y0, x1, y1, نصف قطر الزاوية
    Rect(f32, f32, f32, f32, f32),
    /// نقطة مملوءة
    Dot(f32, f32, f32),
}

use Part::*;

fn parts(icon: Icon) -> &'static [Part] {
    match icon {
        // مجلدات مصفوفة: نفس فكرة أيقونة التطبيق
        Icon::Organize => &[
            Closed(&[(3.0, 8.0), (9.0, 8.0), (11.0, 10.0), (21.0, 10.0), (21.0, 20.0), (3.0, 20.0)]),
            Line(&[(5.0, 5.0), (19.0, 5.0)]),
            Line(&[(8.0, 15.0), (16.0, 15.0)]),
        ],
        Icon::History => &[
            Arc(12.0, 12.0, 8.5, 200.0, 500.0),
            Line(&[(3.2, 5.6), (4.0, 9.1), (7.4, 8.4)]),
            Line(&[(12.0, 7.5), (12.0, 12.0), (15.0, 14.0)]),
        ],
        // منزلقات بدل الترس المعتاد
        Icon::Settings => &[
            Line(&[(4.0, 7.0), (7.0, 7.0)]),
            Circle(9.0, 7.0, 2.0),
            Line(&[(11.0, 7.0), (20.0, 7.0)]),
            Line(&[(4.0, 12.0), (13.0, 12.0)]),
            Circle(15.0, 12.0, 2.0),
            Line(&[(17.0, 12.0), (20.0, 12.0)]),
            Line(&[(4.0, 17.0), (5.0, 17.0)]),
            Circle(7.0, 17.0, 2.0),
            Line(&[(9.0, 17.0), (20.0, 17.0)]),
        ],
        Icon::About => &[Circle(12.0, 12.0, 9.0), Line(&[(12.0, 11.0), (12.0, 16.5)]), Dot(12.0, 7.8, 1.2)],
        Icon::Folder => &[Closed(&[(3.0, 6.0), (9.0, 6.0), (11.0, 8.0), (21.0, 8.0), (21.0, 18.0), (3.0, 18.0)])],
        Icon::Search => &[Circle(10.5, 10.5, 6.0), Line(&[(15.0, 15.0), (20.0, 20.0)])],
        Icon::Check => &[Line(&[(5.0, 12.5), (10.0, 17.5), (19.0, 7.0)])],
        Icon::Close => &[Line(&[(6.0, 6.0), (18.0, 18.0)]), Line(&[(18.0, 6.0), (6.0, 18.0)])],
        Icon::Undo => &[
            Line(&[(8.5, 5.5), (4.0, 10.0), (8.5, 14.5)]),
            Line(&[(4.0, 10.0), (14.0, 10.0)]),
            Arc(14.0, 15.0, 5.0, -90.0, 90.0),
            Line(&[(14.0, 20.0), (8.0, 20.0)]),
        ],
        Icon::Shield => &[
            Closed(&[
                (12.0, 3.0),
                (19.0, 6.0),
                (19.0, 11.0),
                (18.2, 14.8),
                (15.6, 18.4),
                (12.0, 21.0),
                (8.4, 18.4),
                (5.8, 14.8),
                (5.0, 11.0),
                (5.0, 6.0),
            ]),
            Line(&[(9.0, 12.0), (11.2, 14.2), (15.4, 9.8)]),
        ],
        Icon::Person => &[Circle(12.0, 8.0, 3.8), Arc(12.0, 21.5, 7.5, 200.0, 340.0)],
        Icon::Lock => &[
            Rect(5.0, 11.0, 19.0, 20.0, 2.0),
            Arc(12.0, 8.5, 4.0, 180.0, 360.0),
            Line(&[(8.0, 8.5), (8.0, 11.0)]),
            Line(&[(16.0, 8.5), (16.0, 11.0)]),
            Dot(12.0, 15.5, 1.2),
        ],
        Icon::Edit => &[Closed(&[(15.0, 4.0), (20.0, 9.0), (9.0, 20.0), (4.0, 20.0), (4.0, 15.0)]), Line(&[(13.0, 6.0), (18.0, 11.0)])],
        Icon::Trash => &[
            Line(&[(4.0, 7.0), (20.0, 7.0)]),
            Line(&[(9.0, 7.0), (9.0, 4.5), (15.0, 4.5), (15.0, 7.0)]),
            Closed(&[(6.0, 7.0), (7.0, 20.0), (17.0, 20.0), (18.0, 7.0)]),
            Line(&[(10.0, 11.0), (10.0, 16.0)]),
            Line(&[(14.0, 11.0), (14.0, 16.0)]),
        ],
        Icon::Drop => &[
            Line(&[(12.0, 3.0), (12.0, 14.0)]),
            Line(&[(7.5, 9.5), (12.0, 14.0), (16.5, 9.5)]),
            Line(&[(4.0, 14.0), (4.0, 20.0), (20.0, 20.0), (20.0, 14.0)]),
        ],
        Icon::Music => &[Line(&[(9.0, 17.5), (9.0, 5.5), (19.0, 3.8), (19.0, 15.0)]), Circle(6.6, 17.5, 2.4), Circle(16.6, 15.0, 2.4)],
        Icon::Photo => &[
            Rect(3.0, 5.0, 21.0, 19.0, 2.0),
            Circle(8.5, 10.0, 1.6),
            Line(&[(3.6, 17.0), (9.0, 12.0), (13.0, 16.0), (16.0, 13.0), (20.4, 17.4)]),
        ],
        Icon::Video => &[Rect(3.0, 6.0, 16.0, 18.0, 2.0), Closed(&[(16.0, 10.0), (21.0, 7.0), (21.0, 17.0), (16.0, 14.0)])],
        Icon::Document => &[
            Closed(&[(6.0, 3.0), (14.0, 3.0), (19.0, 8.0), (19.0, 21.0), (6.0, 21.0)]),
            Line(&[(14.0, 3.0), (14.0, 8.0), (19.0, 8.0)]),
            Line(&[(9.0, 13.0), (16.0, 13.0)]),
            Line(&[(9.0, 17.0), (16.0, 17.0)]),
        ],
        Icon::Archive => &[
            Rect(3.0, 4.0, 21.0, 9.0, 1.5),
            Line(&[(5.0, 9.0), (5.0, 20.0), (19.0, 20.0), (19.0, 9.0)]),
            Line(&[(10.0, 13.0), (14.0, 13.0)]),
        ],
        Icon::ArrowRight => &[Line(&[(4.0, 12.0), (20.0, 12.0)]), Line(&[(14.0, 6.0), (20.0, 12.0), (14.0, 18.0)])],
        Icon::Warning => &[
            Closed(&[(12.0, 3.5), (21.5, 20.0), (2.5, 20.0)]),
            Line(&[(12.0, 10.0), (12.0, 14.5)]),
            Dot(12.0, 17.2, 1.2),
        ],
        Icon::External => &[
            Line(&[(14.0, 4.0), (20.0, 4.0), (20.0, 10.0)]),
            Line(&[(20.0, 4.0), (11.0, 13.0)]),
            Line(&[(18.0, 14.0), (18.0, 20.0), (4.0, 20.0), (4.0, 6.0), (10.0, 6.0)]),
        ],
        Icon::Clock => &[Circle(12.0, 12.0, 9.0), Line(&[(12.0, 7.0), (12.0, 12.0), (15.5, 14.0)])],
        Icon::Spinner => &[Arc(12.0, 12.0, 8.5, -90.0, 180.0)],
    }
}

fn arc_points(cx: f32, cy: f32, r: f32, from: f32, to: f32) -> Vec<(f32, f32)> {
    let steps = (((to - from).abs() / 6.0).ceil() as usize).max(2);
    (0..=steps)
        .map(|i| {
            let a = (from + (to - from) * i as f32 / steps as f32).to_radians();
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// رسم الأيقونة داخل مربع rect بلون color. rotation (بالراديان) لأيقونة الانتظار فقط
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32, rotation: f32) {
    let scale = rect.width().min(rect.height()) / 24.0;
    let center = rect.center();
    let (sin, cos) = rotation.sin_cos();
    // في الواجهة العربية تنعكس الأيقونات التي لها اتجاه (السهم، التراجع، الفتح خارجياً)
    let mirror = crate::i18n::ar() && matches!(icon, Icon::ArrowRight | Icon::Undo | Icon::External);
    let map = |(x, y): (f32, f32)| -> Pos2 {
        let x = if mirror { 24.0 - x } else { x };
        let (dx, dy) = ((x - 12.0) * scale, (y - 12.0) * scale);
        Pos2::new(center.x + dx * cos - dy * sin, center.y + dx * sin + dy * cos)
    };
    let stroke = Stroke::new((STROKE * scale).max(1.2), color);
    for part in parts(icon) {
        match part {
            Line(points) => {
                painter.add(Shape::line(points.iter().map(|p| map(*p)).collect(), stroke));
            }
            Closed(points) => {
                painter.add(Shape::closed_line(points.iter().map(|p| map(*p)).collect(), stroke));
            }
            Circle(x, y, r) => {
                painter.circle_stroke(map((*x, *y)), r * scale, stroke);
            }
            Arc(x, y, r, from, to) => {
                painter.add(Shape::line(arc_points(*x, *y, *r, *from, *to).into_iter().map(map).collect(), stroke));
            }
            Rect(x0, y0, x1, y1, r) => {
                let rect = egui::Rect::from_two_pos(map((*x0, *y0)), map((*x1, *y1)));
                painter.rect_stroke(rect, r * scale, stroke);
            }
            Dot(x, y, r) => {
                painter.circle_filled(map((*x, *y)), r * scale, color);
            }
        }
    }
}

/// أيقونة كعنصر مستقل في الواجهة
pub fn show(ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    paint(ui.painter(), rect, icon, color, 0.0);
    response
}

/// أيقونة انتظار دوّارة
pub fn spinner(ui: &mut egui::Ui, size: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let t = ui.input(|i| i.time) as f32;
    paint(ui.painter(), rect, Icon::Spinner, color, t * 5.0);
    ui.ctx().request_repaint();
}

/// اسم الملف عند التصدير
#[cfg(test)]
pub fn name(icon: Icon) -> String {
    let debug = format!("{:?}", icon);
    let mut out = String::new();
    for (i, c) in debug.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('-');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// نفس الأيقونة كملف SVG (currentColor يجعلها تأخذ لون النص حولها)
#[cfg(test)]
pub fn to_svg(icon: Icon) -> String {
    let fmt = |v: f32| {
        let s = format!("{:.2}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    let pts = |points: &[(f32, f32)]| points.iter().map(|(x, y)| format!("{},{}", fmt(*x), fmt(*y))).collect::<Vec<_>>().join(" ");
    let mut body = String::new();
    for part in parts(icon) {
        let element = match part {
            Line(points) => format!(r#"<polyline points="{}"/>"#, pts(points)),
            Closed(points) => format!(r#"<polygon points="{}"/>"#, pts(points)),
            Circle(x, y, r) => format!(r#"<circle cx="{}" cy="{}" r="{}"/>"#, fmt(*x), fmt(*y), fmt(*r)),
            Arc(x, y, r, from, to) => {
                let points = arc_points(*x, *y, *r, *from, *to);
                format!(r#"<polyline points="{}"/>"#, pts(&points))
            }
            Rect(x0, y0, x1, y1, r) => format!(
                r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}"/>"#,
                fmt(*x0),
                fmt(*y0),
                fmt(x1 - x0),
                fmt(y1 - y0),
                fmt(*r)
            ),
            Dot(x, y, r) => format!(r#"<circle cx="{}" cy="{}" r="{}" fill="currentColor" stroke="none"/>"#, fmt(*x), fmt(*y), fmt(*r)),
        };
        body.push_str("\n  ");
        body.push_str(&element);
    }
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round">{}
</svg>
"#,
        STROKE, body
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_stays_inside_the_grid() {
        for icon in ALL {
            for part in parts(icon) {
                let points: Vec<(f32, f32)> = match part {
                    Line(p) | Closed(p) => p.to_vec(),
                    Circle(x, y, r) | Dot(x, y, r) => vec![(x - r, y - r), (x + r, y + r)],
                    Arc(x, y, r, from, to) => arc_points(*x, *y, *r, *from, *to),
                    Rect(x0, y0, x1, y1, _) => vec![(*x0, *y0), (*x1, *y1)],
                };
                for (x, y) in points {
                    assert!((1.5..=22.5).contains(&x) && (1.5..=22.5).contains(&y), "{icon:?} leaves the grid at {x},{y}");
                }
            }
        }
    }

    /// يكتب ملفات SVG في assets/icons (cargo test export_icons -- --ignored)
    #[test]
    #[ignore]
    fn export_icons() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
        std::fs::create_dir_all(&dir).unwrap();
        for icon in ALL {
            std::fs::write(dir.join(format!("{}.svg", name(icon))), to_svg(icon)).unwrap();
        }
    }
}
