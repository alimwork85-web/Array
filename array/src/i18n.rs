//! لغة الواجهة: العربية (من اليمين لليسار) أو الإنجليزية. كل نص يظهر للمستخدم يُكتب باللغتين
//! في مكانه: tr("Organize", "تنظيم")، والاختيار من الإعدادات.

use std::sync::atomic::{AtomicBool, Ordering};

static ARABIC: AtomicBool = AtomicBool::new(false);

pub fn set_arabic(on: bool) {
    ARABIC.store(on, Ordering::Relaxed);
}

/// هل الواجهة عربية (من اليمين لليسار)؟
pub fn ar() -> bool {
    ARABIC.load(Ordering::Relaxed)
}

/// النص بلغة الواجهة الحالية
pub fn tr<'a>(en: &'a str, arabic: &'a str) -> &'a str {
    if ar() { arabic } else { en }
}

#[derive(Clone, Copy)]
pub enum Noun {
    File,
    Photo,
    Folder,
    ProtectedFolder,
}

/// صيغ الاسم المعدود بالعربية: واحد، مثنى، جمع (3-10)، مفرد منصوب (11-99)، مفرد (100 وما بعدها)
struct ArabicForms {
    one: &'static str,
    two: &'static str,
    plural: &'static str,
    accusative: &'static str,
    single: &'static str,
}

fn arabic_forms(noun: Noun) -> ArabicForms {
    match noun {
        Noun::File => ArabicForms { one: "ملف واحد", two: "ملفان", plural: "ملفات", accusative: "ملفاً", single: "ملف" },
        Noun::Photo => ArabicForms { one: "صورة واحدة", two: "صورتان", plural: "صور", accusative: "صورة", single: "صورة" },
        Noun::Folder => ArabicForms { one: "مجلد واحد", two: "مجلدان", plural: "مجلدات", accusative: "مجلداً", single: "مجلد" },
        Noun::ProtectedFolder => ArabicForms {
            one: "مجلد محمي واحد",
            two: "مجلدان محميان",
            plural: "مجلدات محمية",
            accusative: "مجلداً محمياً",
            single: "مجلد محمي",
        },
    }
}

fn english_forms(noun: Noun) -> (&'static str, &'static str) {
    match noun {
        Noun::File => ("file", "files"),
        Noun::Photo => ("photo", "photos"),
        Noun::Folder => ("folder", "folders"),
        Noun::ProtectedFolder => ("program or project folder", "program or project folders"),
    }
}

/// العدد مع الاسم بالصيغة الصحيحة للغة المعطاة (أرقام غربية دائماً)
pub fn count_in(arabic: bool, n: usize, noun: Noun) -> String {
    if !arabic {
        let (one, many) = english_forms(noun);
        return if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
    }
    let f = arabic_forms(noun);
    let tens = n % 100;
    match n {
        1 => f.one.to_string(),
        2 => f.two.to_string(),
        _ if (3..=10).contains(&tens) => format!("{n} {}", f.plural),
        _ if (11..=99).contains(&tens) => format!("{n} {}", f.accusative),
        _ => format!("{n} {}", f.single),
    }
}

/// العدد مع الاسم بلغة الواجهة الحالية
pub fn count(n: usize, noun: Noun) -> String {
    count_in(ar(), n, noun)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arabic_counted_nouns() {
        let c = |n| count_in(true, n, Noun::File);
        assert_eq!(c(1), "ملف واحد");
        assert_eq!(c(2), "ملفان");
        assert_eq!(c(3), "3 ملفات");
        assert_eq!(c(10), "10 ملفات");
        assert_eq!(c(11), "11 ملفاً");
        assert_eq!(c(99), "99 ملفاً");
        assert_eq!(c(100), "100 ملف");
        assert_eq!(c(103), "103 ملفات");
        assert_eq!(c(5298), "5298 ملفاً");
        assert_eq!(count_in(true, 1306, Noun::Photo), "1306 صور");
        assert_eq!(count_in(true, 1, Noun::ProtectedFolder), "مجلد محمي واحد");
        assert_eq!(count_in(true, 2, Noun::Folder), "مجلدان");
    }

    #[test]
    fn english_counted_nouns() {
        assert_eq!(count_in(false, 1, Noun::File), "1 file");
        assert_eq!(count_in(false, 7, Noun::File), "7 files");
        assert_eq!(count_in(false, 0, Noun::Photo), "0 photos");
    }
}
