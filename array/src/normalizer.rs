pub fn normalize_arabic(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'أ' | 'إ' | 'آ' | 'ٱ' => 'ا',
            'ة' => 'ه',
            'ى' => 'ي',
            _ => c,
        })
        .collect()
}

/// حذف التشكيل والتطويل حتى تتطابق "اللهُ" و"الله"
pub fn strip_diacritics(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '\u{0610}'..='\u{061A}' | '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{06D6}'..='\u{06ED}' | '\u{0640}'))
        .collect()
}

/// الأرقام العربية الشرقية (٠-٩ و ۰-۹) إلى أرقام غربية 0-9
pub fn to_western_digits(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '٠'..='٩' => char::from(b'0' + (c as u32 - '٠' as u32) as u8),
            '۰'..='۹' => char::from(b'0' + (c as u32 - '۰' as u32) as u8),
            _ => c,
        })
        .collect()
}

fn has_arabic(text: &str) -> bool {
    text.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c))
}

/// بايتات النص كما كانت قبل أن تُقرأ خطأً كـ Windows-1252 / Latin-1
fn latin_bytes(text: &str) -> Option<Vec<u8>> {
    text.chars().map(cp1252_byte).collect()
}

/// إصلاح نص عربي قُرئ بترميز خاطئ (يحدث في وسوم بعض الملفات المنزّلة):
/// - UTF-8 قُرئ كـ Windows-1252: "Ø§Ù„Ø´ÙŠØ®" ← "الشيخ"
/// - Windows-1256 (ترميز ويندوز العربي القديم) قُرئ كـ Latin-1: "ÇáÔíÎ" ← "الشيخ"
pub fn fix_mojibake(text: &str) -> String {
    if has_arabic(text) {
        return text.to_string();
    }
    if text.chars().any(|c| matches!(c, 'Ø' | 'Ù' | 'Ú' | 'Û')) {
        if let Some(fixed) = latin_bytes(text).and_then(|b| String::from_utf8(b).ok()).filter(|f| has_arabic(f)) {
            return fixed;
        }
    }
    if looks_like_cp1256(text) {
        if let Some(bytes) = latin_bytes(text) {
            let fixed: String = bytes.iter().map(|&b| cp1256_char(b)).collect();
            // يُقبل فقط إذا صار أغلب النص حروفاً عربية (حتى لا يُفسد نص فرنسي فيه حروف مشكولة)
            let letters = fixed.chars().filter(|c| c.is_alphabetic()).count();
            let arabic = fixed.chars().filter(|c| ('\u{0621}'..='\u{064A}').contains(c)).count();
            if letters > 0 && arabic * 10 >= letters * 7 {
                return fixed;
            }
        }
    }
    text.to_string()
}

/// نسبة الحروف من نطاق Latin-1 الممتد (À..ÿ) بين كل الحروف
fn extended_ratio(text: &str) -> (usize, usize) {
    let letters: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
    let extended = letters.iter().filter(|c| ('\u{C0}'..='\u{FF}').contains(*c)).count();
    (extended, letters.len())
}

/// نص أغلب حروفه من نطاق Latin-1 الممتد غالباً عربي مكتوب بـ Windows-1256
fn looks_like_cp1256(text: &str) -> bool {
    let (extended, letters) = extended_ratio(text);
    extended >= 3 && extended * 2 >= letters
}

/// هل النص مشوّه بترميز خاطئ لم يمكن إصلاحه؟ (لا يصلح اسماً للملف)
pub fn looks_garbled(text: &str) -> bool {
    let fixed = fix_mojibake(text);
    let (extended, letters) = extended_ratio(&fixed);
    !has_arabic(&fixed) && extended >= 3 && extended * 10 >= letters * 7
}

fn cp1256_char(b: u8) -> char {
    const HIGH: [char; 128] = [
        '€', 'پ', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'ٹ', '‹', 'Œ', 'چ', 'ژ', 'ڈ',
        'گ', '‘', '’', '“', '”', '•', '–', '—', 'ک', '™', 'ڑ', '›', 'œ', '\u{200C}', '\u{200D}', 'ں',
        '\u{A0}', '،', '¢', '£', '¤', '¥', '¦', '§', '¨', '©', 'ھ', '«', '¬', '\u{AD}', '®', '¯',
        '°', '±', '²', '³', '´', 'µ', '¶', '·', '¸', '¹', '؛', '»', '¼', '½', '¾', '؟',
        'ہ', 'ء', 'آ', 'أ', 'ؤ', 'إ', 'ئ', 'ا', 'ب', 'ة', 'ت', 'ث', 'ج', 'ح', 'خ', 'د',
        'ذ', 'ر', 'ز', 'س', 'ش', 'ص', 'ض', '×', 'ط', 'ظ', 'ع', 'غ', 'ـ', 'ف', 'ق', 'ك',
        'à', 'ل', 'â', 'م', 'ن', 'ه', 'و', 'ç', 'è', 'é', 'ê', 'ë', 'ى', 'ي', 'î', 'ï',
        'ً', 'ٌ', 'ٍ', 'َ', 'ô', 'ُ', 'ِ', '÷', 'ّ', 'ù', 'ْ', 'û', 'ü', '\u{200E}', '\u{200F}', 'ے',
    ];
    if b < 0x80 { b as char } else { HIGH[(b - 0x80) as usize] }
}

fn cp1252_byte(c: char) -> Option<u8> {
    let code = c as u32;
    // ASCII و Latin-1، والبايتات غير المعرّفة في 1252 تظهر كرموز تحكم U+0080..U+009F
    if code <= 0xFF {
        return Some(code as u8);
    }
    const MAP: [(char, u8); 27] = [
        ('€', 0x80), ('‚', 0x82), ('ƒ', 0x83), ('„', 0x84), ('…', 0x85), ('†', 0x86), ('‡', 0x87),
        ('ˆ', 0x88), ('‰', 0x89), ('Š', 0x8A), ('‹', 0x8B), ('Œ', 0x8C), ('Ž', 0x8E), ('‘', 0x91),
        ('’', 0x92), ('“', 0x93), ('”', 0x94), ('•', 0x95), ('–', 0x96), ('—', 0x97), ('˜', 0x98),
        ('™', 0x99), ('š', 0x9A), ('›', 0x9B), ('œ', 0x9C), ('ž', 0x9E), ('Ÿ', 0x9F),
    ];
    MAP.iter().find(|(m, _)| *m == c).map(|(_, b)| *b)
}

pub fn sanitize_filename(name: &str) -> String {
    let invalid_chars = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let cleaned: String = name
        .chars()
        .filter(|c| !invalid_chars.contains(c) && !c.is_control())
        .collect();

    // ويندوز يحذف النقاط والمسافات في نهاية اسم الملف أو المجلد بصمت
    let trimmed = cleaned.trim().trim_end_matches('.').trim().to_string();
    if trimmed.is_empty() {
        "Unknown".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_garbled_arabic() {
        assert_eq!(fix_mojibake("Ø§Ù„Ø´ÙŠØ® Ø¹Ù„ÙŠ"), "الشيخ علي");
        // حرف "ف" بايته الثاني 0x81 غير معرّف في 1252 ويظهر كرمز تحكم
        assert_eq!(fix_mojibake("Ù\u{81}ÙŠ"), "في");
        assert_eq!(fix_mojibake("Café Ø"), "Café Ø");
        assert_eq!(fix_mojibake("عبدالله"), "عبدالله");
        // ترميز ويندوز العربي القديم (من ملف حقيقي عند المستخدم)
        assert_eq!(fix_mojibake("ÑÓÇáÉ ãä ÇáÔíÎ Úáì ÇáÞÑäì"), "رسالة من الشيخ على القرنى");
        // نص فرنسي عادي لا يُعتبر مشوّهاً
        assert_eq!(fix_mojibake("Café crème"), "Café crème");
        assert!(!looks_garbled("Café crème"));
        assert!(!looks_garbled("Small Tone"));
        assert_eq!(fix_mojibake("Été à côté"), "Été à côté");
    }

    #[test]
    fn digits_and_diacritics() {
        assert_eq!(to_western_digits("١٤٤٠هـ"), "1440هـ");
        assert_eq!(strip_diacritics("اللهُ بضرٍ"), "الله بضر");
    }

    #[test]
    fn sanitize_removes_trailing_dots() {
        assert_eq!(sanitize_filename("name..."), "name");
        assert_eq!(sanitize_filename("a/b"), "ab");
    }
}
