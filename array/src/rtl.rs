use std::sync::LazyLock;

use regex::Regex;
use unicode_bidi::{BidiInfo, Level};

static RE_NUMBER_RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[0-9]+(?:[-:./][0-9]+)+").unwrap());

fn has_arabic(text: &str) -> bool {
    text.chars().any(|c| matches!(c, '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}'))
}

/// egui لا يصل الحروف العربية ولا يرتبها من اليمين لليسار، فنحوّل النص إلى أشكال
/// الحروف المتصلة ثم نرتبه بصرياً.
/// كل جزء بين \ يُعالج وحده بحيث تبقى أجزاء المسار بترتيبها من اليسار لليمين،
/// واتجاه كل جزء يُحدد من أول حرف فيه (الاسم العربي يُقرأ من اليمين).
pub fn for_display(text: &str) -> String {
    if !has_arabic(text) {
        return text.to_string();
    }
    text.split('\\').map(shape_segment).collect::<Vec<_>>().join("\\")
}

fn shape_segment(segment: &str) -> String {
    if !has_arabic(segment) {
        return segment.to_string();
    }
    // أرقام مفصولة بـ - أو : أو . أو / (تواريخ وأوقات) تُعزل لتبقى بترتيبها من اليسار لليمين،
    // وإلا عرض النص العربي "2026-03-18" بالمقلوب "18-03-2026"
    let isolated = RE_NUMBER_RUN.replace_all(segment, "\u{2066}$0\u{2069}");
    let shaped = ar_reshaper::reshape_line(&isolated);
    let bidi = BidiInfo::new(&shaped, None::<Level>);
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
