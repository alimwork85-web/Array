//! أسماء جديدة للملفات التي لا يدل اسمها على شيء (أكواد مثل "4749d951]"، نصوص عشوائية،
//! أو أسماء عامة مثل "YouTube" و"Untitled").
//!
//! القاعدة الذهبية: إن لم يوجد مصدر موثوق للاسم فلا تسمية. المصادر بالترتيب:
//! EXIF للصور، تاريخ الإنشاء داخل الفيديو، وسوم lofty للصوت، العنوان الداخلي للمستند،
//! ثم التاريخ المكتوب في الاسم نفسه. تاريخ آخر تعديل للملف ليس مصدراً أبداً.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use chrono::NaiveDateTime;

use std::sync::LazyLock;

use regex::Regex;

use crate::normalizer::{fix_mojibake, normalize_arabic, sanitize_filename, to_western_digits};

/// تاريخ ووقت داخل الاسم: "20260409_145348"، "2025-05-29 18.23.30"، "IMG_20240101_093000"
static RE_NAME_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[^0-9])((?:19|20)[0-9]{2})[-_.]?(0[1-9]|1[0-2])[-_.]?(0[1-9]|[12][0-9]|3[01])(?:[ _T-]?([01][0-9]|2[0-3])[-_.:]?([0-5][0-9])(?:[-_.:]?([0-5][0-9]))?)?(?:[^0-9]|$)").unwrap()
});

/// التاريخ والوقت المكتوبان في الاسم نفسه (غالباً وقت التصوير)، بصيغة "2026-04-09 14-53"
fn date_from_name(name: &str) -> Option<String> {
    let name = to_western_digits(name);
    let c = RE_NAME_DATE.captures(&name)?;
    let mut date = format!("{}-{}-{}", &c[1], &c[2], &c[3]);
    if let (Some(h), Some(m)) = (c.get(4), c.get(5)) {
        date.push_str(&format!(" {}-{}", h.as_str(), m.as_str()));
    }
    Some(date)
}

/// السنة المكتوبة في الاسم نفسه ("20260219_183536_IMG_2755" → 2026)
pub fn year_from_name(name: &str) -> Option<i32> {
    RE_NAME_DATE.captures(&to_western_digits(name))?[1].parse().ok()
}

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
    // سداسي عشري فيه أرقام وحروف (أجزاء UUID مثل "88aa" و"ab22")، أو طويل
    let all_hex = token.chars().all(|c| c.is_ascii_hexdigit());
    let is_hex = all_hex && (token.len() >= 6 || (token.len() >= 3 && letters > 0 && digits > 0));
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

/// نوع الملف كما يهم التسمية (لا علاقة له بمجلدات الفئات)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Image,
    Video,
    Audio,
    Document,
    Other,
}

/// مجموعات التسمية الست؛ لكل مجموعة مفتاح تشغيل مستقل في `Options`
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NamingGroup {
    /// أسماء رديئة (`is_junk_name`)
    Junk,
    /// صور ومقاطع واتساب (`IMG-YYYYMMDD-WA####` و`VID-…-WA…`)
    Whatsapp,
    /// لقطات الشاشة (`Screenshot_…` و`لقطة شاشة …`)
    Screenshot,
    /// أسماء الكاميرا (`IMG_` و`DSC_` و`VID_` و`PXL_` و`DCIM`)
    Camera,
    /// تسجيلات صوتية
    Recording,
    /// مستندات لها عنوان داخلي
    Document,
}

/// من أين جاء التاريخ. تاريخ آخر تعديل للملف ليس مصدراً أبداً.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DateSource {
    Exif,
    Embedded,
    FromName,
}

static RE_WHATSAPP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(?:IMG|VID)-\d{8}-WA\d+").unwrap());
static RE_SCREENSHOT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\s*(?:screenshot|screen shot|لقطة شاشة)").unwrap());
static RE_CAMERA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(?:IMG|DSC|VID|PXL|DCIM)[ \-_]?\d").unwrap());
/// عناوين القوالب: "Microsoft Word - Document1"
static RE_TEMPLATE_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*microsoft\s+(?:word|excel|powerpoint)\s*-\s*").unwrap());
static RE_DC_TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<dc:title[^>]*>(.*?)</dc:title>").unwrap());

/// مجموعة الملف إن كان من نوع تستهدفه التسمية؛ None = اسمه يبقى كما هو
pub fn naming_group(path: &Path, stem: &str, kind: Kind) -> Option<NamingGroup> {
    let stem = to_western_digits(stem);
    if RE_WHATSAPP.is_match(&stem) {
        return Some(NamingGroup::Whatsapp);
    }
    if RE_SCREENSHOT.is_match(&stem) {
        return Some(NamingGroup::Screenshot);
    }
    if RE_CAMERA.is_match(&stem) {
        return Some(NamingGroup::Camera);
    }
    if !is_junk_name(&stem) {
        return None;
    }
    // المستند يدخل مجموعة العنوان الداخلي فقط إذا كان يحتاج اسماً أصلاً
    if kind == Kind::Document && document_title(path).is_some() {
        return Some(NamingGroup::Document);
    }
    if kind == Kind::Audio {
        return Some(NamingGroup::Recording);
    }
    Some(NamingGroup::Junk)
}

/// التاريخ من مصدر موثوق فقط، مع مصدره
pub fn trusted_date(path: &Path, kind: Kind) -> Option<(String, DateSource)> {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    match kind {
        Kind::Image => exif_datetime(path)
            .map(|d| (d, DateSource::Exif))
            .or_else(|| date_from_name(&stem).map(|d| (d, DateSource::FromName))),
        // تاريخ الإنشاء داخل الفيديو أدق من وقت النسخ أو التعديل
        Kind::Video => crate::video::creation_time(path)
            .map(|t| (t.format("%Y-%m-%d %H-%M-%S").to_string(), DateSource::Embedded))
            .or_else(|| date_from_name(&stem).map(|d| (d, DateSource::FromName))),
        Kind::Audio => date_from_name(&stem).map(|d| (d, DateSource::FromName)),
        _ => None,
    }
}

/// تركيب الاسم: التاريخ أولاً ثم الوسم. لا مدة، ولا تاريخ تعديل.
pub fn join_name(date: &str, tag: Option<&str>) -> String {
    match tag.map(str::trim).filter(|t| !t.is_empty()) {
        Some(tag) => format!("{date} {tag}"),
        None => date.to_string(),
    }
}

/// DateTimeOriginal من EXIF بصيغة "2024-05-12 14-30-05"
fn exif_datetime(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let exif = exif::Reader::new().read_from_container(&mut reader).ok()?;
    let field = exif.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)?;
    // قيمة EXIF نصية وتظهر بين علامتي تنصيص: "2024:05:12 14:30:05"
    let value = field.display_value().to_string();
    let text = value.trim().trim_matches('"');
    let dt = NaiveDateTime::parse_from_str(text, "%Y:%m:%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S"))
        .ok()?;
    Some(dt.format("%Y-%m-%d %H-%M-%S").to_string())
}

/// العنوان الداخلي للمستند (خصائص PDF، أو docProps/core.xml في DOCX وXLSX وPPTX).
/// عناوين القوالب وكل ما يكشفه `is_junk_name` يُرفض.
pub fn document_title(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    let raw = match ext.as_str() {
        "pdf" => pdf_title(path)?,
        "docx" | "xlsx" | "pptx" => zip_entry_text(path, "docProps/core.xml")?,
        "odt" | "ods" | "odp" => zip_entry_text(path, "meta.xml")?,
        _ => return None,
    };
    let fixed = fix_mojibake(raw.trim());
    let title = RE_TEMPLATE_TITLE.replace(&fixed, "");
    let clean = sanitize_filename(title.trim());
    (!clean.is_empty() && !is_junk_name(&clean)).then_some(clean)
}

/// أول ميجابايت من أول الملف وآخر ميجابايت منه (قاموس /Info قد يكون في الطرفين)
fn pdf_title(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut bytes = Vec::new();
    file.by_ref().take(1_000_000).read_to_end(&mut bytes).ok()?;
    if len > 1_000_000 {
        file.seek(SeekFrom::Start(len - 1_000_000)).ok()?;
        let mut tail = Vec::new();
        file.by_ref().take(1_000_000).read_to_end(&mut tail).ok()?;
        bytes.extend_from_slice(&tail);
    }
    find_pdf_title(&bytes)
}

fn find_pdf_title(bytes: &[u8]) -> Option<String> {
    let mut from = 0;
    while let Some(pos) = find_sub(&bytes[from..], b"/Title").map(|p| p + from) {
        from = pos + 6;
        let mut i = from;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let raw = match bytes.get(i) {
            Some(b'(') => pdf_literal(bytes, i + 1),
            Some(b'<') => pdf_hex(bytes, i + 1),
            _ => None,
        };
        if let Some(text) = raw.map(|b| pdf_text(&b)).filter(|s| !s.trim().is_empty()) {
            return Some(text);
        }
    }
    None
}

fn find_sub(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// نص PDF بين قوسين: يفك \n \( \) \\ و\ddd وينتهي عند قوس غير مهرَّب
fn pdf_literal(bytes: &[u8], mut i: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut depth = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                let next = *bytes.get(i + 1)?;
                match next {
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(0x0D),
                    b't' => out.push(b'\t'),
                    b'0'..=b'7' => {
                        let mut value = 0u32;
                        let mut used = 0;
                        while used < 3 {
                            match bytes.get(i + 1 + used) {
                                Some(d @ b'0'..=b'7') => value = value * 8 + (d - b'0') as u32,
                                _ => break,
                            }
                            used += 1;
                        }
                        out.push(value as u8);
                        i += used;
                    }
                    other => out.push(other),
                }
                i += 2;
            }
            b'(' => {
                depth += 1;
                out.push(b'(');
                i += 1;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(out);
                }
                out.push(b')');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    None
}

/// نص PDF بالنظام الست عشري: <FEFF062A0642...>
fn pdf_hex(bytes: &[u8], mut i: usize) -> Option<Vec<u8>> {
    let mut digits = Vec::new();
    while i < bytes.len() {
        match bytes[i] {
            b'>' => break,
            b if b.is_ascii_hexdigit() => digits.push(b),
            _ => {}
        }
        i += 1;
    }
    let nibble = |b: u8| (b as char).to_digit(16).unwrap_or(0) as u8;
    Some(digits.chunks(2).map(|c| (nibble(c[0]) << 4) | c.get(1).map(|b| nibble(*b)).unwrap_or(0)).collect())
}

/// ترميز نص PDF: UTF-16 عند وسم BOM، وإلا PDFDocEncoding (يُقرأ هنا كـ Latin-1)
fn pdf_text(bytes: &[u8]) -> String {
    let utf16 = |rest: &[u8], big_endian: bool| -> String {
        let units: Vec<u16> = rest
            .chunks(2)
            .map(|c| {
                let pair = [c[0], *c.get(1).unwrap_or(&0)];
                if big_endian {
                    u16::from_be_bytes(pair)
                } else {
                    u16::from_le_bytes(pair)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFE, 0xFF, rest @ ..] => utf16(rest, true),
        [0xFF, 0xFE, rest @ ..] => utf16(rest, false),
        _ => bytes.iter().map(|b| *b as char).collect(),
    }
}

/// نص عنصر داخل ملف أوفيس (ZIP)
fn zip_entry_text(path: &Path, entry: &str) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut xml = String::new();
    archive.by_name(entry).ok()?.read_to_string(&mut xml).ok()?;
    let caps = RE_DC_TITLE.captures(&xml)?;
    Some(xml_unescape(caps.get(1)?.as_str()))
}

fn xml_unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
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
            "03b510f8-88aa-4487-ab22-bd6b93174921",
            "88462d6e-60cf-42e5-beae-ebb71ed18c4f",
            "133649_466985853345771_200500525",
            "٢٠٢٦٠٤٠٩_١٤٥٣٤٨",
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
    fn dates_are_read_from_the_old_name() {
        assert_eq!(date_from_name("٢٠٢٦٠٤٠٩_١٤٥٣٤٨ [IMA-da4c15b4]").as_deref(), Some("2026-04-09 14-53"));
        assert_eq!(date_from_name("20260519_122023").as_deref(), Some("2026-05-19 12-20"));
        assert_eq!(date_from_name("IMG_20240101_093000").as_deref(), Some("2024-01-01 09-30"));
        assert_eq!(date_from_name("صورة - 2026-07-28").as_deref(), Some("2026-07-28"));
        // أرقام فيسبوك الطويلة ليست تاريخاً
        assert_eq!(date_from_name("133649_466985853345771_200500525"), None);
    }

    #[test]
    fn names_are_date_first_without_duration() {
        assert_eq!(join_name("2024-05-12 14-30-05", Some("واتساب")), "2024-05-12 14-30-05 واتساب");
        assert_eq!(join_name("2024-05-12 14-30-05", Some("IMG_2353")), "2024-05-12 14-30-05 IMG_2353");
        assert_eq!(join_name("2024-05-12 14-30", None), "2024-05-12 14-30");
        // لا وسم فارغ ولا مسافة زائدة
        assert_eq!(join_name("2024-05-12 14-30", Some("  ")), "2024-05-12 14-30");
    }

    #[test]
    fn groups_are_recognized_before_anything_else() {
        assert_eq!(naming_group(Path::new("x.jpg"), "IMG-20240512-WA0003", Kind::Image), Some(NamingGroup::Whatsapp));
        assert_eq!(naming_group(Path::new("x.mp4"), "VID-20240512-WA0003", Kind::Video), Some(NamingGroup::Whatsapp));
        assert_eq!(naming_group(Path::new("x.png"), "Screenshot 2024-05-12 143005", Kind::Image), Some(NamingGroup::Screenshot));
        assert_eq!(naming_group(Path::new("x.png"), "لقطة شاشة 2024-05-12", Kind::Image), Some(NamingGroup::Screenshot));
        assert_eq!(naming_group(Path::new("x.jpg"), "IMG 2353", Kind::Image), Some(NamingGroup::Camera));
        assert_eq!(naming_group(Path::new("x.jpg"), "DSC_0042", Kind::Image), Some(NamingGroup::Camera));
        assert_eq!(naming_group(Path::new("x.jpg"), "4749d951", Kind::Image), Some(NamingGroup::Junk));
        assert_eq!(naming_group(Path::new("x.mp3"), "4749d951", Kind::Audio), Some(NamingGroup::Recording));
        // الاسم المفهوم لا يدخل أي مجموعة
        assert_eq!(naming_group(Path::new("x.jpg"), "trip to jeddah", Kind::Image), None);
    }

    #[test]
    fn only_a_trusted_date_counts() {
        let root = crate::organizer::tests::temp_tree("trusted_date", &["a.jpg", "b.mp3"]);
        // ملف بلا EXIF وبلا تاريخ في الاسم: لا تاريخ، ولو كان تاريخ التعديل موجوداً
        assert_eq!(trusted_date(&root.join("a.jpg"), Kind::Image), None);
        assert_eq!(trusted_date(&root.join("b.mp3"), Kind::Audio), None);
        // تاريخ الاسم مقبول
        assert_eq!(
            trusted_date(Path::new("20240512_143005.jpg"), Kind::Image),
            Some(("2024-05-12 14-30".to_string(), DateSource::FromName))
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn exif_date_is_read_from_the_photo() {
        let root = crate::organizer::tests::temp_tree("exif_date", &[]);
        let photo = root.join("IMG_2353.jpg");
        crate::organizer::tests::jpeg_with_date(&photo, "2024:05:12 14:30:05");
        assert_eq!(
            trusted_date(&photo, Kind::Image),
            Some(("2024-05-12 14-30-05".to_string(), DateSource::Exif))
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn template_titles_are_rejected() {
        let root = crate::organizer::tests::temp_tree("pdf_titles", &[]);
        let template = root.join("ملف.pdf");
        crate::organizer::tests::pdf_with_title(&template, "Microsoft Word - Document1");
        assert_eq!(document_title(&template), None);
        let real = root.join("تقرير.pdf");
        crate::organizer::tests::pdf_with_title(&real, "Annual Report 2024");
        assert_eq!(document_title(&real).as_deref(), Some("Annual Report 2024"));
        // ملف بلا عنوان داخلي
        let none = root.join("بلا.pdf");
        std::fs::write(&none, b"%PDF-1.4\n%%EOF\n").unwrap();
        assert_eq!(document_title(&none), None);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
