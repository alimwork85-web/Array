use std::sync::LazyLock;

use regex::Regex;
use unicode_bidi::{BidiInfo, Level};

use crate::normalizer::to_western_digits;

static RE_NUMBER_RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[0-9]+(?:[-:./][0-9]+)+").unwrap());

fn has_arabic(text: &str) -> bool {
    text.chars().any(|c| matches!(c, '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}'))
}

/// egui لا يصل الحروف العربية ولا يرتبها من اليمين لليسار، فنحوّل النص إلى أشكال
/// الحروف المتصلة ثم نرتبه بصرياً.
/// كل جزء بين \ يُعالج وحده بحيث تبقى أجزاء المسار بترتيبها من اليسار لليمين،
/// واتجاه كل جزء يُحدد من أول حرف فيه (الاسم العربي يُقرأ من اليمين).
pub fn for_display(text: &str) -> String {
    // صيغة واحدة للأرقام في كل التطبيق: الأرقام الغربية 0-9، حتى لو كانت في اسم الملف شرقية
    let text = to_western_digits(text);
    if !has_arabic(&text) {
        return text;
    }
    text.split('\\').map(|s| shape_segment(s, None)).collect::<Vec<_>>().join("\\")
}

/// جملة من الواجهة العربية: اتجاهها الأساسي من اليمين لليسار حتى لو بدأت بكلمة إنجليزية
/// ("Array يرتّب ملفاتك")، ولا تُقسم عند \ لأنها ليست مساراً
pub fn rtl_line(text: &str) -> String {
    let text = to_western_digits(text);
    if !has_arabic(&text) {
        return text;
    }
    shape_segment(&text, Some(Level::rtl()))
}

/// تقسيم نص إلى أسطر بترتيبه المنطقي قبل عكسه للعرض؛ الالتفاف بعد العكس يضع آخر الجملة
/// العربية في السطر الأول. measure يعيد عرض السطر كما سيُرسم
pub fn wrap_lines(text: &str, max_width: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if !line.is_empty() && measure(&candidate) > max_width {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    lines
}

fn shape_segment(segment: &str, base: Option<Level>) -> String {
    if !has_arabic(segment) {
        return segment.to_string();
    }
    // أرقام مفصولة بـ - أو : أو . أو / (تواريخ وأوقات) تُعزل لتبقى بترتيبها من اليسار لليمين،
    // وإلا عرض النص العربي "2026-03-18" بالمقلوب "18-03-2026"
    let isolated = RE_NUMBER_RUN.replace_all(segment, "\u{2066}$0\u{2069}");
    let shaped = ar_reshaper::reshape_line(&isolated);
    let bidi = BidiInfo::new(&shaped, base);
    let mut out = String::with_capacity(shaped.len());
    for para in &bidi.paragraphs {
        let (levels, runs) = bidi.visual_runs(para, para.range.clone());
        for run in runs {
            let text: String = shaped[run.clone()].chars().filter(|c| !matches!(c, '\u{2066}'..='\u{2069}')).collect();
            let text = text.as_str();
            if levels[run.start].is_rtl() {
                // عكس المقطع ومرآة الأقواس، لأن egui يرسم الحروف كما هي بلا مرآة
                out.extend(text.chars().rev().map(mirror));
            } else {
                out.push_str(text);
            }
        }
    }
    out
}

fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_text_is_unchanged() {
        assert_eq!(for_display("C:\\Music\\song.mp3"), "C:\\Music\\song.mp3");
    }

    #[test]
    fn arabic_word_is_joined_and_reversed() {
        // "سلام": يُعرض بأشكال متصلة ومعكوس الترتيب (م في اليسار)
        let shown = for_display("سلام");
        assert!(!shown.contains('س'), "letters should be in joined forms: {shown:?}");
        assert_eq!(shown.chars().count(), 3); // "لا" تتحول إلى حرف مركب واحد
        assert_eq!(shown.chars().next(), Some('\u{FEE1}')); // م في آخر الكلمة
    }

    #[test]
    fn path_keeps_ltr_order_around_arabic() {
        let shown = for_display("D:\\موسيقى\\a");
        assert!(shown.starts_with("D:\\") && shown.ends_with("\\a"), "{shown:?}");
    }

    #[test]
    fn dates_inside_arabic_names_keep_their_order() {
        let shown = for_display("صورة أرشيفية - 2026-03-18");
        assert!(shown.contains("2026-03-18"), "{shown:?}");
        assert!(!shown.contains('\u{2066}') && !shown.contains('\u{2069}'), "{shown:?}");
    }

    #[test]
    fn only_western_digits_are_shown() {
        let shown = for_display("عبدالله الموسى ١٤٤٠هـ");
        assert!(shown.contains("1440"), "{shown:?}");
        assert!(!shown.chars().any(|c| ('\u{0660}'..='\u{0669}').contains(&c)), "{shown:?}");
        assert!(rtl_line("الملف ٣").contains('3'));
    }

    #[test]
    fn interface_sentence_starting_with_english_reads_right_to_left() {
        // "Array" في بداية الجملة تبقى في أقصى اليمين بعد العرض
        let shown = rtl_line("Array يرتّب ملفاتك");
        assert!(shown.ends_with("Array"), "{shown:?}");
    }

    #[test]
    fn wrapping_happens_in_reading_order() {
        let lines = wrap_lines("واحد اثنان ثلاثة أربعة", 12.0, |s| s.chars().count() as f32);
        assert_eq!(lines, ["واحد اثنان", "ثلاثة أربعة"]);
    }

    #[test]
    fn arabic_path_parts_keep_their_order() {
        let first = for_display("موسيقى");
        let second = for_display("عربي");
        let shown = for_display("D:\\موسيقى\\عربي (1).mp3");
        assert!(shown.starts_with(&format!("D:\\{}\\", first)), "{shown:?}");
        // الجزء الأخير يُقرأ من اليمين: الاسم ثم (1) ثم الامتداد
        assert!(shown.ends_with(&second), "{shown:?}");
        assert!(shown.contains("mp3.(1)"), "{shown:?}");
    }
}
