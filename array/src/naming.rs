//! أسماء جديدة للملفات التي لا يدل اسمها على شيء (أكواد مثل "4749d951]"، نصوص عشوائية،
//! أو أسماء عامة مثل "YouTube" و"Untitled"): يُبنى الاسم من سياق الملف وتاريخه ومدته.

use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Local};
use lofty::file::AudioFile;
use lofty::probe::Probe;

use crate::normalizer::normalize_arabic;

/// أسماء عامة لا تميّز الملف. ما بعد "=" هو النص الذي يصلح سياقاً للاسم الجديد (فارغ = لا يصلح)
const PLACEHOLDERS: [(&str, &str); 18] = [
    ("untitled", ""),
    ("youtube", ""),
    ("unknown", ""),
    ("audio", ""),
    ("audio track", ""),
    ("track", ""),
    ("video", ""),
    ("sound", ""),
    ("new", ""),
    ("file", ""),
    ("mp3", ""),
    ("recording", "Recording"),
    ("new recording", "Recording"),
    ("voice", "Voice recording"),
    ("voice recording", "Voice recording"),
    ("تسجيل", "تسجيل صوتي"),
    ("تسجيل صوتي", "تسجيل صوتي"),
    ("مقطع صوتي", "مقطع صوتي"),
];

fn is_arabic(c: char) -> bool {
    ('\u{0600}'..='\u{06FF}').contains(&c)
}

/// كلمة تبدو كوداً أو نصاً عشوائياً: "4749d951"، "H7Wb7eOxN"، "jxX"
fn is_random_token(token: &str) -> bool {
    if token.chars().any(is_arabic) {
        return false;
    }
    let letters = token.chars().filter(|c| c.is_alphabetic()).count();
    let digits = token.chars().filter(|c| c.is_ascii_digit()).count();
    let is_hex = token.len() >= 6 && token.chars().all(|c| c.is_ascii_hexdigit());
    // حروف كبيرة وسط الكلمة بعد حروف صغيرة أكثر من مرة: خلط عشوائي للحالة
    let chars: Vec<char> = token.chars().collect();
    let case_jumps = chars.windows(2).filter(|w| w[0].is_lowercase() && w[1].is_uppercase()).count();
    is_hex || (letters > 0 && digits > 0 && token.len() >= 5) || (case_jumps >= 1 && letters >= 3 && token.len() <= 4) || case_jumps >= 2
}

fn words(name: &str) -> Vec<String> {
    name.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect()
}

/// النص العام إن كان الاسم مجرد اسم عام ("YouTube (1)"، "09-04 - تسجيل صوتي")
fn placeholder(name: &str) -> Option<&'static str> {
    let reduced = words(&normalize_arabic(&name.to_lowercase()))
        .into_iter()
        .filter(|w| !w.chars().all(|c| c.is_ascii_digit()))
        .collect::<Vec<_>>()
        .join(" ");
    PLACEHOLDERS
        .iter()
        .find(|(p, _)| normalize_arabic(p) == reduced)
        .map(|(_, context)| *context)
}

/// هل الاسم لا يعبّر عن الملف؟ (أقل من 40% من حروفه في كلمات مفهومة، أو اسم عام)
pub fn is_junk_name(name: &str) -> bool {
    if placeholder(name).is_some() {
        return true;
    }
    let tokens = words(name);
    let total: usize = tokens.iter().map(|t| t.chars().filter(|c| c.is_alphabetic()).count()).sum();
    let meaningful: usize = tokens
        .iter()
        .filter(|t| !is_random_token(t))
        .map(|t| t.chars().filter(|c| c.is_alphabetic()).count())
        .filter(|&letters| letters >= 2)
        .sum();
    // لا كلمة مفهومة واحدة (حرفان فأكثر)، أو أغلب الحروف في كلمات عشوائية
    meaningful == 0 || meaningful * 10 < total * 4
}

/// سياق من الاسم العام نفسه إن كان مفيداً ("تسجيل صوتي")
pub fn placeholder_context(name: &str) -> Option<String> {
    placeholder(name).filter(|c| !c.is_empty()).map(|c| c.to_string())
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 { format!("{}h{:02}m{:02}s", h, m, s) } else { format!("{}m{:02}s", m, s) }
}

/// اسم جديد: "السياق - تاريخ التنزيل ووقته - المدة"، مثل "house lo - 2026-09-12 23-31 - 0m07s"
pub fn generated_name(context: &str, path: &Path) -> String {
    let mut parts = vec![context.trim().to_string()];
    if let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) {
        let local: DateTime<Local> = modified.into();
        parts.push(local.format("%Y-%m-%d %H-%M").to_string());
    }
    let duration = Probe::open(path).and_then(|p| p.read()).map(|f| f.properties().duration()).ok();
    if let Some(d) = duration.filter(|d| !d.is_zero()) {
        parts.push(format_duration(d));
    }
    parts.retain(|p| !p.is_empty());
    parts.join(" - ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_meaningless_names() {
        for junk in [
            "4749d951]",
            "0750707c] (1)",
            "1 (1)",
            "YouTube (1)",
            "Untitled - 1",
            "-H7Wb7eOxN-mx6trNVI2 EBsCu2zNMG O jxX 5WG4u-bW1 0zWrVYdMVxShmi1YheykpznRs1-faAlke0QjV",
            "09-04 - تسجيل صوتي",
            "a",
        ] {
            assert!(is_junk_name(junk), "{junk} should be junk");
        }
    }

    #[test]
    fn keeps_real_names() {
        for good in [
            "Small Tone",
            "Cut T2",
            "TvQuran.com 143",
            "Ma Yehtaj",
            "IMG 001",
            "عبدالله الموسى 1440هـ",
            "house lo",
            "الحلقة 19 - الشيخ محمد",
            "Punch Face",
            "CV",
            "ID",
            "رد",
        ] {
            assert!(!is_junk_name(good), "{good} should be kept");
        }
    }

    #[test]
    fn placeholder_text_can_be_context() {
        assert_eq!(placeholder_context("09-04 - تسجيل صوتي").as_deref(), Some("تسجيل صوتي"));
        assert_eq!(placeholder_context("YouTube (1)"), None);
    }

    #[test]
    fn durations_are_readable() {
        assert_eq!(format_duration(Duration::from_secs(259)), "4m19s");
        assert_eq!(format_duration(Duration::from_secs(4205)), "1h10m05s");
    }

    #[test]
    fn generated_name_has_context_and_date() {
        let root = crate::organizer::tests::temp_tree("gen_name", &["x.mp3"]);
        let name = generated_name("house lo", &root.join("x.mp3"));
        assert!(name.starts_with("house lo - 20"), "{name}");
        // أرقام غربية فقط
        assert!(!name.chars().any(|c| ('\u{0660}'..='\u{0669}').contains(&c)), "{name}");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
