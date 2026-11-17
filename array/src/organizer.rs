use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::LazyLock;
use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use lofty::tag::Accessor;
use regex::Regex;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::i18n::tr;
use crate::grouping::{self, Assignment, Track, DEFAULT_AI_MARKERS};
use crate::naming::{self, Kind, NamingGroup};
use crate::normalizer::{fix_mojibake, looks_garbled, normalize_arabic, sanitize_filename, to_western_digits};
use crate::safety::SafetyChecker;
use crate::video;

// تُجمَّع التعبيرات مرة واحدة بدلاً من كل ملف
// ضوضاء الأسماء فقط: أكواد بين [ ] ومنها [AUD-...] المبتورة، عدادات التكرار (1)، وكلمات مثل (Official Video).
// الأقواس ذات المعنى (اسم الآية أو السورة) تبقى.
static RE_NOISE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\[[^\]]*\]|\[(?:AUD|VID)\b[^\]]*$|\(\s*\d+\s*\)|\((?:official[^)]*|lyrics?|audio|video|hd|4k|كلمات|فيديو كليب|حصري[اًا]?)\)|\bAUD-\w+\b|[_#]").unwrap()
});
static RE_PIPES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[|｜]+").unwrap());
static RE_SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static RE_INNER_EXT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\.(mp3|m4a|aac|wav|ogg|flac|wma|opus|mp4|mkv|avi|mov|jpe?g|png|pdf|docx?)\s*$").unwrap());
static RE_EPISODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(الحلقة|حلقة|part|ep|episode)\s*\d+").unwrap());

/// مجلد كل فئة داخل المجلد المختار: المفتاح الداخلي ثم الاسم العربي الافتراضي
const CATEGORY_FOLDERS: [(&str, &str); 7] = [
    ("Music", "صوتيات"),
    ("Pictures", "صور"),
    ("Videos", "فيديو"),
    ("Documents", "مستندات"),
    ("Archives", "مضغوطات"),
    ("Installers", "مثبّتات"),
    ("Recordings", "تسجيلات"),
];

/// تُقسم السنة إلى مجلدات أشهر إذا تجاوز عدد ملفاتها هذا الحد (ثابت واحد، بلا خيار للمستخدم)
const YEAR_SPLIT_LIMIT: usize = 500;

/// مجلد النسخ المكررة للمراجعة: لا حذف أبداً، والتراجع يعيدها
const DUPLICATES_FOLDER: &str = "مكررات للمراجعة";

/// ملفات ترافق ملفاً رئيسياً: الترجمة مع الفيديو، وبيانات التعديل مع الصورة
const COMPANION_EXTENSIONS: [&str; 5] = ["srt", "ass", "vtt", "xmp", "aae"];

/// كلمات تدل على صاحب المقطع في نهاية اسم الملف (مثل: "محاضرة - الشيخ فلان")
pub const DEFAULT_SPEAKER_KEYWORDS: [&str; 4] = ["الشيخ", "دكتور", "فنان", "أدهم"];

/// إعدادات التنظيم، وتُحفظ كما هي في settings.json
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    /// تنظيف أسماء الملفات (حذف الأقواس والرموز)؛ عند false تبقى الأسماء الأصلية
    pub rename_files: bool,
    pub speaker_keywords: Vec<String>,
    /// كلمات تدل على مقاطع الذكاء الاصطناعي (اسم الأداة مثل gemini أو suno)
    pub ai_markers: Vec<String>,
    /// الملفات الموجودة داخل مجلدات فرعية تبقى كما هي (لا تُنقل ولا يُعاد تسميتها)؛
    /// فقط الملفات المتفرقة في المجلد المختار تُنظَّم. عند false تُفرَّغ كل المجلدات ويُعاد بناؤها.
    pub keep_existing_folders: bool,
    /// الفيديو المتفرق يُرتَّب في مجلدات سنوات داخل مجلد الفيديو (السنة من الاسم أو من بيانات الملف)
    pub videos_by_year: bool,
    /// مظهر الواجهة: "system" يتبع ويندوز، أو "light" أو "dark"
    pub appearance: String,
    /// لغة الواجهة: "ar" (عربي، من اليمين لليسار) أو "en"
    pub language: String,
    /// هل عُرضت شاشة الترحيب من قبل
    pub welcome_seen: bool,
    /// مجموعة «أسماء رديئة»: ملفات اسمه لا يدل عليها تُسمّى من مصدرها الموثوق
    pub rename_junk_names: bool,
    /// مجموعة «صور ومقاطع واتساب»
    pub rename_whatsapp_media: bool,
    /// مجموعة «لقطات الشاشة»
    pub rename_screenshots: bool,
    /// مجموعة «أسماء الكاميرا»
    pub rename_camera_names: bool,
    /// مجموعة «تسجيلات صوتية»
    pub rename_recordings: bool,
    /// مجموعة «مستندات لها عنوان داخلي»
    pub rename_documents_by_title: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            rename_files: true,
            speaker_keywords: DEFAULT_SPEAKER_KEYWORDS.iter().map(|k| k.to_string()).collect(),
            ai_markers: DEFAULT_AI_MARKERS.iter().map(|k| k.to_string()).collect(),
            keep_existing_folders: true,
            videos_by_year: true,
            appearance: "system".to_string(),
            language: "ar".to_string(),
            welcome_seen: false,
            rename_junk_names: true,
            rename_whatsapp_media: true,
            rename_screenshots: true,
            rename_camera_names: true,
            rename_recordings: true,
            rename_documents_by_title: true,
        }
    }
}

impl Options {
    /// هل مجموعة التسمية هذه مفعّلة؟
    fn naming_enabled(&self, group: NamingGroup) -> bool {
        match group {
            NamingGroup::Junk => self.rename_junk_names,
            NamingGroup::Whatsapp => self.rename_whatsapp_media,
            NamingGroup::Screenshot => self.rename_screenshots,
            NamingGroup::Camera => self.rename_camera_names,
            NamingGroup::Recording => self.rename_recordings,
            NamingGroup::Document => self.rename_documents_by_title,
        }
    }
}

/// نقل ملف واحد: من مكانه الأصلي إلى مجلد فئته داخل المجلد المختار
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlannedMove {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// بيان المجلد: سجل كتابة مسبقة داخل المجلد المرتَّب نفسه (`.array/history.json`).
/// يُكتب قبل تنفيذ أي عملية ويُعلَّم بعد نجاحها، فيبقى التراجع ممكنًا بعد حذف سجل AppData.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Journal {
    pub source: PathBuf,
    /// مجلدات أنشأها التنظيم (تُحذف عند التراجع إذا فرغت)
    #[serde(default)]
    pub created_dirs: Vec<PathBuf>,
    pub entries: Vec<JournalEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JournalEntry {
    pub from: PathBuf,
    pub to: PathBuf,
    /// true = نُقلت العملية فعلاً
    #[serde(default)]
    pub done: bool,
}

/// مجلد بيانات Array داخل المجلد المرتَّب
pub const JOURNAL_DIR: &str = ".array";

impl Journal {
    pub fn path(source: &Path) -> PathBuf {
        source.join(JOURNAL_DIR).join("history.json")
    }

    pub fn exists(source: &Path) -> bool {
        Self::path(source).is_file()
    }

    pub fn new(source: &Path, moves: &[PlannedMove], created_dirs: &[PathBuf]) -> Self {
        Self {
            source: source.to_path_buf(),
            created_dirs: created_dirs.to_vec(),
            entries: moves.iter().map(|m| JournalEntry { from: m.from.clone(), to: m.to.clone(), done: false }).collect(),
        }
    }

    /// يُعاد كتابة الملف كاملاً بعد كل عملية (بسيط ومقبول لحجم مجلد عادي؛
    /// لو صار عدد الملفات بالآلاف فالحل الإلحاقي سطر بسطر)
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path(&self.source);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("{}: {}", tr("Could not write the record", "تعذّر كتابة البيان"), e))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&path, text).map_err(|e| format!("{}: {}", tr("Could not write the record", "تعذّر كتابة البيان"), e))
    }

    pub fn load(source: &Path) -> Option<Self> {
        serde_json::from_str(&fs::read_to_string(Self::path(source)).ok()?).ok()
    }
}

/// خطة التنظيم الكاملة قبل نقل أي ملف (تُعرض للمعاينة ثم تُنفَّذ)
pub struct Plan {
    pub source: PathBuf,
    pub moves: Vec<PlannedMove>,
    /// مجلدات ستُنشأ (غير موجودة الآن)؛ التراجع يحذفها إذا فرغت ولا يمس المجلدات الأصلية
    pub new_dirs: Vec<PathBuf>,
    /// ملفات موجودة أصلاً في مكانها الصحيح وباسمها الصحيح فلا تتحرك
    pub in_place: usize,
    /// ملفات داخل مجلدات المستخدم الموجودة تُركت كما هي (خيار keep_existing_folders)
    pub kept_in_folders: usize,
    /// عدد النسخ المكررة التي ستُنقل إلى «مكررات للمراجعة»
    pub duplicates: usize,
    /// مجلدات برامج أو مشاريع لم يُلمس أي شيء داخلها، مع السبب
    pub skipped: Vec<(PathBuf, String)>,
}

/// نتيجة التنفيذ أو التراجع
#[derive(Debug, Default)]
pub struct RunResult {
    pub done: Vec<PlannedMove>,
    pub failed: Vec<PlannedMove>,
    /// في التراجع: ملفات كانت في مكانها الأصلي أصلاً (تنظيم توقف قبل نقلها)
    pub unchanged: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileType {
    Audio,
    Image,
    Video,
    Document,
    Archive,
    /// مثبّت في جذر المجلد المختار فقط (.exe و.msi)
    Installer,
    Other,
}

struct RawFileInfo {
    path: PathBuf,
    category: String,
    /// رقم الملف في قائمة الأدلة الصوتية (للموسيقى فقط)
    track_index: Option<usize>,
    /// اسم الملف الجديد بدون الامتداد (منظَّف أو الأصلي حسب الإعدادات)
    new_stem: String,
    /// نوع الملف كما يهم التسمية
    kind: Kind,
    /// مجموعة التسمية؛ None يعني أن الاسم يبقى كما هو
    naming_group: Option<NamingGroup>,
    /// اسم من وسوم الصوت ("الفنان - العنوان") إن كانت الوسوم صالحة
    audio_tag_name: Option<String>,
    /// سياق من الاسم العام نفسه ("تسجيل صوتي") إن وُجد
    placeholder_context: Option<String>,
    extension: String,
}

pub struct OrganizerEngine;

impl OrganizerEngine {
    pub fn get_file_type(ext: &str) -> FileType {
        match ext.to_lowercase().as_str() {
            "mp3" | "flac" | "m4a" | "m4b" | "wav" | "aac" | "ogg" | "wma" | "opus" | "aiff" | "aif" | "amr" => FileType::Audio,
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "heic" | "heif" | "tif" | "tiff" | "cr2" | "nef" | "arw" | "dng" => FileType::Image,
            "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "m4v" | "3gp" | "mpg" | "mpeg" => FileType::Video,
            "pdf" | "docx" | "doc" | "xlsx" | "xls" | "pptx" | "ppt" | "odt" | "ods" | "odp" | "rtf" | "txt" | "epub" | "mobi" | "azw3" => {
                FileType::Document
            }
            "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" => FileType::Archive,
            "exe" | "msi" => FileType::Installer,
            // أي نوع آخر (برامج، اختصارات، إعدادات، قواعد بيانات، ملفات غير معروفة) لا يُلمس أبداً
            _ => FileType::Other,
        }
    }

    pub(crate) fn clean_string(input: &str) -> String {
        let input = to_western_digits(&fix_mojibake(input));
        let mut cleaned = RE_NOISE.replace_all(&input, " ").to_string();
        cleaned = RE_PIPES.replace_all(&cleaned, " ").to_string();
        cleaned = RE_SPACES.replace_all(&cleaned, " ").to_string();
        // امتداد مكرر داخل الاسم: "أغنية.mp3 (1).mp3" → "أغنية"
        cleaned = RE_INNER_EXT.replace(cleaned.trim(), "").to_string();
        // شرطات ونقاط زائدة في البداية أو النهاية: "- وصية" → "وصية"
        cleaned = cleaned.trim_matches(|c: char| c == '-' || c == '.' || c.is_whitespace()).to_string();

        sanitize_filename(&cleaned)
    }

    /// مفتاح المقارنة: توحيد الحروف العربية وحالة الأحرف اللاتينية
    fn group_key(name: &str) -> String {
        normalize_arabic(name).to_lowercase()
    }

    /// كشف السلاسل والحلقات (مثل: الحلقة 19، حلقة 20...)
    fn is_episode(text: &str) -> bool {
        RE_EPISODE.is_match(text) || text.starts_with("الحلقة") || text.starts_with("حلقة")
    }

    /// استخراج المجموعات والسلاسل الذكي؛ keywords يجب أن تكون مطبَّعة بـ group_key
    fn extract_group_key(artist_raw: &str, title_raw: &str, keywords: &[String]) -> (String, String) {
        let clean_tit = Self::clean_string(title_raw);
        let series = ("سلسلة الحلقات".to_string(), Self::group_key("سلسلة الحلقات"));

        // اسم ملف مثل "الحلقة 19 - الشيخ فلان" يُقسم إلى فنان="الحلقة 19" وعنوان="الشيخ فلان"
        if Self::is_episode(artist_raw) {
            let norm_tit = Self::group_key(&clean_tit);
            if norm_tit.chars().count() > 3 {
                return (clean_tit, norm_tit);
            }
            return series;
        }

        let clean_art = if artist_raw.trim().is_empty() { String::new() } else { Self::clean_string(artist_raw) };
        let norm_art = Self::group_key(&clean_art);

        // إذا كان اسم الفنان موجوداً وصحيحاً (ليس كوداً أو مجهولاً)
        let is_valid_artist = norm_art != "single tracks"
            && norm_art != "unknown"
            && norm_art != "track"
            && norm_art.chars().count() > 2;

        if is_valid_artist {
            return (clean_art, norm_art);
        }

        if Self::is_episode(&clean_tit) {
            // البحث عن اسم الشيخ أو السلسلة في نهاية العنوان بعد الشرطة (-)
            if let Some(pos) = clean_tit.rfind('-') {
                let suffix = clean_tit[pos + 1..].trim();
                let norm_suf = Self::group_key(suffix);
                if norm_suf.chars().count() > 3 {
                    return (suffix.to_string(), norm_suf);
                }
            }
            return series;
        }

        // البحث عن أصحاب المقاطع في نهاية اسم الملف (المقارنة بعد التطبيع: أ ← ا)
        if let Some(pos) = clean_tit.rfind('-') {
            let suffix = clean_tit[pos + 1..].trim();
            let norm_suf = Self::group_key(suffix);
            if norm_suf.chars().count() > 3 && keywords.iter().any(|k| !k.is_empty() && norm_suf.contains(k.as_str())) {
                return (suffix.to_string(), norm_suf);
            }
        }

        ("Single Tracks".to_string(), "single tracks".to_string())
    }

    fn move_file(src: &Path, dst: &Path) -> std::io::Result<()> {
        // fs::rename على ويندوز يستبدل الملف الموجود، فلا نكتب أبداً فوق ملف قائم
        if dst.exists() {
            return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "destination exists"));
        }
        if fs::rename(src, dst).is_ok() {
            return Ok(());
        }
        // النسخ ثم الحذف؛ عند أي فشل نحذف النسخة حتى لا يبقى ملف ناقص أو مكرر
        let result = fs::copy(src, dst).and_then(|_| fs::remove_file(src));
        if result.is_err() {
            let _ = fs::remove_file(dst);
        }
        result
    }

    /// حذف المجلدات التي فرغت، صعوداً حتى المجلد المختار (ولا يُحذف هو أبداً).
    /// allowed يحدد أي مجلد يجوز حذفه، و remove_dir يرفض أي مجلد غير فارغ.
    fn remove_emptied_dirs<'a>(files: impl Iterator<Item = &'a PathBuf>, source_path: &Path, allowed: impl Fn(&Path) -> bool) {
        for file in files {
            let mut dir = file.parent();
            while let Some(d) = dir {
                if d == source_path || !d.starts_with(source_path) || !allowed(d) || fs::remove_dir(d).is_err() {
                    break;
                }
                dir = d.parent();
            }
        }
    }

    fn check_source(source_path: &Path) -> Result<(), String> {
        if !source_path.is_dir() {
            return Err(tr("The selected folder does not exist.", "المجلد المختار غير موجود.").to_string());
        }
        SafetyChecker::check_root(source_path)
    }

    /// أسماء مجلدات موجودة تُعتبر مجلد الفئة (تُقارن بعد التطبيع: حالة الأحرف والهمزات والتاء المربوطة)
    fn category_synonyms(category: &str) -> &'static [&'static str] {
        match category {
            "Music" => &["music", "songs", "audio", "موسيقى", "الموسيقى", "أغاني", "الأغاني", "صوتيات"],
            "Pictures" => &["pictures", "photos", "photo", "images", "image", "صور", "الصور", "صورة", "الصورة"],
            "Videos" => &["videos", "video", "movies", "فيديو", "فيديوهات", "الفيديو", "مقاطع فيديو", "أفلام"],
            "Documents" => &["documents", "docs", "مستندات", "المستندات", "وثائق"],
            "Installers" => &["installers", "setup", "مثبتات", "المثبتات", "مثبّتات"],
            "Recordings" => &["recordings", "voice recordings", "تسجيلات", "التسجيلات", "تسجيل صوتي"],
            _ => &["archives", "compressed", "أرشيف", "مضغوطات", "ملفات مضغوطة"],
        }
    }

    /// مجلد كل فئة داخل المجلد المختار: مجلد موجود يناسبها إن وُجد، وإلا مجلد جديد بالاسم العربي
    fn category_dirs(source_path: &Path) -> HashMap<&'static str, PathBuf> {
        let mut existing: Vec<(String, PathBuf)> = fs::read_dir(source_path)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| e.metadata().is_ok_and(|m| m.is_dir() && !SafetyChecker::is_hidden_or_system(&m)))
                    .filter(|e| !SafetyChecker::is_skipped_dir_name(&e.file_name().to_string_lossy()))
                    .filter(|e| SafetyChecker::program_folder(&e.path()).is_none())
                    .map(|e| (Self::group_key(e.file_name().to_string_lossy().trim()), e.path()))
                    .collect()
            })
            .unwrap_or_default();
        existing.sort();

        // المجلد المختار نفسه قد يكون مجلد فئة (مثل D:\Music): يُستخدم مباشرة بدل إنشاء Music داخله
        let root_key = Self::group_key(source_path.file_name().unwrap_or_default().to_string_lossy().trim());

        let mut dirs = HashMap::new();
        for (category, default_name) in CATEGORY_FOLDERS {
            let synonyms: Vec<String> = Self::category_synonyms(category).iter().map(|s| Self::group_key(s)).collect();
            if synonyms.contains(&root_key) {
                dirs.insert(category, source_path.to_path_buf());
                continue;
            }
            // الاسم الإنجليزي أولاً ثم بقية المرادفات بترتيبها
            let found = synonyms.iter().find_map(|syn| existing.iter().find(|(key, _)| key == syn).map(|(_, p)| p.clone()));
            let dir = found.unwrap_or_else(|| {
                let default = source_path.join(default_name);
                // اسم الفئة محجوز لملف أو لمجلد برنامج: نستخدم اسماً آخر بدل الكتابة داخله
                if default.exists() { source_path.join(format!("{default_name} (Array)")) } else { default }
            });
            dirs.insert(category, dir);
        }
        dirs
    }

    /// كل الملفات المعروفة الأنواع التي سيتم نقلها، ومجلدات البرامج والمشاريع التي تُركت مع السبب
    fn collect_files(source_path: &Path) -> (Vec<PathBuf>, Vec<(PathBuf, String)>) {
        let mut skipped: Vec<(PathBuf, String)> = Vec::new();

        // تخطّي المخفي وملفات النظام والروابط، ومجلدات البرامج والمشاريع بكل ما فيها
        let walker = WalkDir::new(source_path).into_iter().filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            if e.metadata().map_or(true, |m| SafetyChecker::is_hidden_or_system(&m)) {
                return false;
            }
            if !e.file_type().is_dir() {
                return true;
            }
            if SafetyChecker::is_skipped_dir_name(&e.file_name().to_string_lossy()) {
                return false;
            }
            if let Some(kind) = SafetyChecker::program_folder(e.path()) {
                skipped.push((e.path().to_path_buf(), kind.reason()));
                return false;
            }
            true
        });

        let files = walker
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| {
                let Some(ext) = e.path().extension().and_then(|x| x.to_str()).map(|x| x.to_lowercase()) else {
                    return false;
                };
                match Self::get_file_type(&ext) {
                    // مرافق لملف رئيسي (ترجمة، بيانات تعديل) يُجمع ليُنقل معه
                    FileType::Other => COMPANION_EXTENSIONS.contains(&ext.as_str()),
                    // المثبّتات تُنقل من جذر المجلد المختار فقط، لا من المجلدات الفرعية
                    FileType::Installer => e.depth() == 1,
                    _ => true,
                }
            })
            .map(|e| e.into_path())
            .collect();
        (files, skipped)
    }

    /// حساب وجهة كل ملف بدون نقل أي شيء. كل الملفات في المجلدات الفرعية تُجمع ثم تُوزَّع
    /// على مجلدات الفئات داخل المجلد المختار نفسه (الموجودة أو الجديدة).
    #[cfg(test)]
    pub fn plan(source_path: &Path, options: &Options) -> Result<Plan, String> {
        Self::plan_with(source_path, options)
    }

    pub fn plan_with(source_path: &Path, options: &Options) -> Result<Plan, String> {
        Self::check_source(source_path)?;
        let keywords: Vec<String> = options.speaker_keywords.iter().map(|k| Self::group_key(k.trim())).collect();

        let (all_files, skipped) = Self::collect_files(source_path);
        // المرافق (ترجمة/بيانات تعديل/فيديو صورة حية) يُنقل مع رئيسيه لا وحده
        let (files, companions) = Self::split_packages(&all_files);
        let category_dirs = Self::category_dirs(source_path);
        let music_dir = &category_dirs["Music"];
        let mut raw_files: Vec<RawFileInfo> = Vec::new();
        let mut tracks: Vec<Track> = Vec::new();

        for path in files {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_string();
            let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
            let file_type = Self::get_file_type(&ext);
            let kind = Self::naming_kind(&file_type);
            let (category, clean_name, track, audio_tag_name) = match file_type {
                FileType::Audio => {
                    let (artist, title, tag_title) = Self::extract_audio_info(&path);
                    let clean = Self::best_name(&stem, tag_title.clone());
                    let tag_name = tag_title.clone().map(|t| Self::tag_name(&artist, &t));
                    // تسجيل صوتي اسمه لا يدل عليه: مجلد التسجيلات لا الموسيقى
                    if naming::naming_group(&path, &clean, Kind::Audio) == Some(NamingGroup::Recording) {
                        ("Recordings", clean, None, tag_name)
                    } else {
                        let (speaker, speaker_key) = Self::extract_group_key(&artist, &title, &keywords);
                        let speaker = (speaker_key != "single tracks").then_some(speaker);
                        let track = grouping::read_track(&path, source_path, music_dir, &artist, &title, speaker.as_deref(), &options.ai_markers);
                        ("Music", clean, Some(track), tag_name)
                    }
                }
                other => {
                    let category = match other {
                        FileType::Image => "Pictures",
                        FileType::Video => "Videos",
                        FileType::Document => "Documents",
                        FileType::Installer => "Installers",
                        _ => "Archives",
                    };
                    (category, Self::clean_string(&stem), None, None)
                }
            };
            // بدون إعادة تسمية يبقى الاسم والامتداد كما هما تماماً
            let (new_stem, extension) = if options.rename_files {
                (clean_name, ext.to_lowercase())
            } else {
                (stem, ext)
            };

            let naming_group = options.rename_files.then(|| naming::naming_group(&path, &new_stem, kind)).flatten();
            let placeholder_context = naming::placeholder_context(&new_stem);
            let track_index = track.map(|t| {
                tracks.push(t);
                tracks.len() - 1
            });
            raw_files.push(RawFileInfo {
                path,
                category: category.to_string(),
                track_index,
                new_stem,
                kind,
                naming_group,
                audio_tag_name,
                placeholder_context,
                extension,
            });
        }

        // تجميع الموسيقى بمقارنة الأدلة بين كل الملفات
        let existing_music_dirs: Vec<String> = fs::read_dir(music_dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default();
        let assignments = grouping::assign(&tracks, &existing_music_dirs);
        let misc_dir = music_dir.join(grouping::misc_folder_name(&existing_music_dirs));
        let ai_dir = music_dir.join(grouping::ai_folder_name(&existing_music_dirs));
        // مجلدات المستخدم الموجودة لا تُلمس: ملفاتها تبقى كما هي بالاسم والمكان
        let kept = |path: &Path| -> bool {
            let Ok(relative) = path.strip_prefix(source_path) else { return false };
            let mut parts = relative.components();
            let first = parts.next().map(|c| c.as_os_str().to_string_lossy().trim().to_lowercase());
            let inside_subfolder = parts.next().is_some();
            // مجلدات أنشأتها نسخ قديمة من البرنامج ليست تنظيماً للمستخدم
            inside_subfolder && !matches!(first.as_deref(), Some("single tracks" | "unknown" | "array_organized"))
        };
        let (raw_files, kept_files): (Vec<RawFileInfo>, Vec<RawFileInfo>) =
            raw_files.into_iter().partition(|f| !(options.keep_existing_folders && kept(&f.path)));
        let kept_in_folders = kept_files.len();

        // سنة كل صورة/فيديو/تسجيل (لاختيار المجلد فقط)، ثم عدّها لكل سنة لتقسيم الزائد إلى أشهر
        let folder_dates: Vec<Option<(i32, u32)>> = raw_files
            .iter()
            .map(|f| match f.category.as_str() {
                "Pictures" | "Videos" | "Recordings" => naming::folder_date(&f.path, f.kind),
                _ => None,
            })
            .collect();
        let mut year_counts: HashMap<(String, i32), usize> = HashMap::new();
        for (f, date) in raw_files.iter().zip(&folder_dates) {
            if let Some((year, _)) = date {
                *year_counts.entry((f.category.clone(), *year)).or_default() += 1;
            }
        }
        // السنوات الزائدة على الحد تُقسم إلى أشهر
        let split_years: HashSet<(String, i32)> = year_counts
            .iter()
            .filter(|(_, count)| **count > YEAR_SPLIT_LIMIT)
            .map(|((category, year), _)| (category.clone(), *year))
            .collect();
        let existing_years: HashMap<&str, Vec<String>> = ["Pictures", "Videos", "Recordings"]
            .into_iter()
            .map(|category| {
                let names = fs::read_dir(&category_dirs[category])
                    .map(|entries| {
                        entries
                            .flatten()
                            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                            .map(|e| e.file_name().to_string_lossy().to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                (category, names)
            })
            .collect();

        let targeted: Vec<(RawFileInfo, PathBuf)> = raw_files
            .into_iter()
            .zip(folder_dates)
            .map(|(mut f, date)| {
                let dir = match f.track_index.map(|i| &assignments[i]) {
                    Some(Assignment::Folder(name)) => music_dir.join(name),
                    Some(Assignment::Ai) => ai_dir.clone(),
                    Some(Assignment::Misc) => misc_dir.clone(),
                    None => match f.category.as_str() {
                        "Pictures" | "Recordings" => Self::year_dir(&category_dirs[f.category.as_str()], &existing_years, &split_years, date, f.category.as_str()),
                        "Videos" if options.videos_by_year => Self::year_dir(&category_dirs["Videos"], &existing_years, &split_years, date, "Videos"),
                        // المستندات تُقسم فرعياً بنوعها
                        "Documents" => category_dirs["Documents"].join(Self::document_folder(&f.extension)),
                        other => category_dirs[other].clone(),
                    },
                };
                if let Some(name) = Self::trusted_name(&f, options) {
                    f.new_stem = name;
                }
                (f, dir)
            })
            .collect();

        let (mut moves, in_place, finals, duplicates) = Self::plan_moves(targeted, source_path);
        // المرافق يتبع رئيسيه: نفس المجلد ونفس الاسم الجديد
        let mut reserved: HashSet<String> = moves.iter().map(|m| m.to.to_string_lossy().to_lowercase()).collect();
        for (companion, master) in companions {
            let Some(dest) = finals.get(&master) else { continue };
            let raw_ext = companion.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
            let ext = if options.rename_files { raw_ext.to_lowercase() } else { raw_ext };
            let target = Self::free_path(&dest.with_extension(ext), &mut reserved);
            if target.to_string_lossy().to_lowercase() != companion.to_string_lossy().to_lowercase() {
                moves.push(PlannedMove { from: companion, to: target });
            }
        }

        // المجلدات غير الموجودة التي سيُنشئها التنفيذ
        let mut new_dirs: Vec<PathBuf> = Vec::new();
        for m in &moves {
            let mut dir = m.to.parent();
            while let Some(d) = dir {
                if d == source_path || !d.starts_with(source_path) || d.exists() || new_dirs.iter().any(|n| n == d) {
                    break;
                }
                new_dirs.push(d.to_path_buf());
                dir = d.parent();
            }
        }
        new_dirs.sort();

        Ok(Plan {
            source: source_path.to_path_buf(),
            moves,
            new_dirs,
            in_place,
            kept_in_folders,
            duplicates,
            skipped,
        })
    }

    /// مجلد السنة داخل الفئة: يُعاد استخدام مجلد سنة موجود، ويُقسَّم إلى أشهر
    /// إذا تجاوزت السنة الحد الثابت (بلا خيار للمستخدم)
    fn year_dir(
        category_dir: &Path,
        existing: &HashMap<&str, Vec<String>>,
        split_years: &HashSet<(String, i32)>,
        date: Option<(i32, u32)>,
        category: &str,
    ) -> PathBuf {
        let Some((year, month)) = date else { return category_dir.to_path_buf() };
        let names = existing.get(category).map(|v| v.as_slice()).unwrap_or(&[]);
        let dir = category_dir.join(video::year_folder_name(year, names));
        if split_years.contains(&(category.to_string(), year)) {
            return dir.join(format!("{month:02}"));
        }
        dir
    }

    /// مجلد المستند بحسب نوعه
    fn document_folder(ext: &str) -> &'static str {
        match ext.to_lowercase().as_str() {
            "pdf" => "PDF",
            "doc" | "docx" | "rtf" | "odt" => "Word",
            "xls" | "xlsx" | "ods" => "Excel",
            "ppt" | "pptx" | "odp" => "عروض",
            "epub" | "mobi" | "azw3" => "كتب إلكترونية",
            _ => "نصوص",
        }
    }

    /// فصل الملفات الرئيسية عن مرافقاتها: الترجمة مع الفيديو، وبيانات التعديل مع الصورة،
    /// وفيديو الصورة الحية (.mov) مع صورة heic. المرافق بلا رئيسي يبقى مكانه ولا يُجمع.
    fn split_packages(files: &[PathBuf]) -> (Vec<PathBuf>, Vec<(PathBuf, PathBuf)>) {
        let key = |p: &Path| -> Option<(PathBuf, String)> {
            Some((p.parent()?.to_path_buf(), p.file_stem()?.to_string_lossy().to_lowercase()))
        };
        let ext = |p: &Path| p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let mut by_key: HashMap<(PathBuf, String), Vec<&PathBuf>> = HashMap::new();
        for path in files {
            if let Some(k) = key(path) {
                by_key.entry(k).or_default().push(path);
            }
        }

        let mut masters = Vec::new();
        let mut companions = Vec::new();
        for path in files {
            let e = ext(path);
            let siblings = key(path).and_then(|k| by_key.get(&k).cloned()).unwrap_or_default();
            let find = |wanted: FileType| -> Option<PathBuf> {
                siblings.iter().find(|s| Self::get_file_type(&ext(s)) == wanted).map(|s| (*s).clone())
            };
            // صورة حية: فيديو الصورة الحية بصورة heic بنفس الاسم
            let live_photo = || siblings.iter().find(|s| matches!(ext(s).as_str(), "heic" | "heif")).map(|s| (*s).clone());
            let is_companion_ext = COMPANION_EXTENSIONS.contains(&e.as_str());
            let master = match e.as_str() {
                "srt" | "ass" | "vtt" => find(FileType::Video),
                "xmp" | "aae" => find(FileType::Image),
                "mov" | "m4v" => live_photo(),
                _ => None,
            };
            match master {
                Some(m) => companions.push((path.clone(), m)),
                // مرافق بلا رئيسي: يبقى مكانه ولا يُنقل
                None if is_companion_ext => {}
                None => masters.push(path.clone()),
            }
        }
        (masters, companions)
    }

    /// مسار غير مستعمل (لا ملف عليه ولا محجوز في الخطة) بلاحقة (2) فما بعد
    fn free_path(path: &Path, reserved: &mut HashSet<String>) -> PathBuf {
        let lower = |p: &Path| p.to_string_lossy().to_lowercase();
        if !path.exists() && !reserved.contains(&lower(path)) {
            reserved.insert(lower(path));
            return path.to_path_buf();
        }
        let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let ext = path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let mut n = 2;
        loop {
            let candidate = if ext.is_empty() {
                dir.join(format!("{stem} ({n})"))
            } else {
                dir.join(format!("{stem} ({n}).{ext}"))
            };
            if !candidate.exists() && !reserved.contains(&lower(&candidate)) {
                reserved.insert(lower(&candidate));
                return candidate;
            }
            n += 1;
        }
    }

    pub fn execute(plan: &Plan, progress: &AtomicUsize) -> RunResult {
        let mut result = RunResult::default();
        // البيان يُكتب قبل أول عملية: لو انقطع التنفيذ يُعرف ما تحرّك وما لم يتحرك
        let mut journal = Journal::new(&plan.source, &plan.moves, &plan.new_dirs);
        let _ = journal.save();
        let lower = |p: &Path| p.to_string_lossy().to_lowercase();
        // أهداف تطابق مصادر أخرى تعني تبديلاً (photo.jpg ↔ photo (1).jpg):
        // تُنقل هذه الملفات أولاً إلى اسم مؤقت ثم إلى هدفها، فلا يتبادل ملفان اسميهما.
        let sources: HashSet<String> = plan.moves.iter().map(|m| lower(&m.from)).collect();
        let mut staged: Vec<(PlannedMove, PathBuf)> = Vec::new();
        for m in &plan.moves {
            let needs_temp = sources.contains(&lower(&m.to)) && m.to.exists();
            let src = if needs_temp {
                let tmp = Self::temp_name(&m.from);
                if Self::move_file(&m.from, &tmp).is_ok() { tmp } else { m.from.clone() }
            } else {
                m.from.clone()
            };
            staged.push((m.clone(), src));
        }
        for (m, src) in staged {
            let ok = m.to.parent().is_some_and(|dir| fs::create_dir_all(dir).is_ok()) && Self::move_file(&src, &m.to).is_ok();
            if ok {
                if let Some(entry) = journal.entries.iter_mut().find(|e| e.from == m.from && e.to == m.to) {
                    entry.done = true;
                }
                let _ = journal.save();
                result.done.push(m.clone());
            } else {
                // فشل: أعِد الملف من الاسم المؤقت إلى مكانه الأصلي
                if src != m.from {
                    let _ = Self::move_file(&src, &m.from);
                }
                result.failed.push(m.clone());
            }
            progress.fetch_add(1, Ordering::Relaxed);
        }
        Self::remove_emptied_dirs(result.done.iter().map(|m| &m.from), &plan.source, |_| true);
        result
    }

    /// اسم مؤقت بجانب الملف لكسر التبديل بين اسمين
    fn temp_name(path: &Path) -> PathBuf {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        path.with_file_name(format!(".{name}.arraytmp"))
    }

    /// إرجاع الملفات إلى أماكنها الأصلية. لا يكتب فوق أي ملف موجود.
    /// يُحذف بعدها فقط ما أنشأه التنظيم من مجلدات (created_dirs) إذا فرغ؛ المجلدات الأصلية تبقى.
    pub fn undo(moves: &[PlannedMove], source_path: &Path, created_dirs: &[PathBuf], progress: &AtomicUsize) -> RunResult {
        let mut result = RunResult::default();
        for m in moves.iter().rev() {
            progress.fetch_add(1, Ordering::Relaxed);
            if !m.to.exists() && m.from.exists() {
                // لم يُنقل أصلاً (تنظيم توقف في المنتصف) أو أُرجع في تراجع سابق
                result.unchanged += 1;
                continue;
            }
            let ok = m.from.parent().is_some_and(|dir| fs::create_dir_all(dir).is_ok()) && Self::move_file(&m.to, &m.from).is_ok();
            if ok { result.done.push(m.clone()) } else { result.failed.push(m.clone()) }
        }
        Self::remove_emptied_dirs(result.done.iter().map(|m| &m.to), source_path, |d| created_dirs.iter().any(|c| c == d));
        result
    }

    /// هل يوجد في المجلد بيان تنظيم سابق؟
    pub fn has_journal(source_path: &Path) -> bool {
        Journal::exists(source_path)
    }

    /// تراجع من البيان وحده: يعمل بعد حذف سجل AppData أو إعادة تثبيت البرنامج.
    /// يعيد فقط ما نُقل فعلاً ولا يكتب فوق أي ملف.
    pub fn undo_from_journal(source_path: &Path, progress: &AtomicUsize) -> Option<RunResult> {
        let mut journal = Journal::load(source_path)?;
        let moves: Vec<PlannedMove> = journal
            .entries
            .iter()
            .filter(|e| e.done)
            .map(|e| PlannedMove { from: e.from.clone(), to: e.to.clone() })
            .collect();
        if moves.is_empty() {
            return Some(RunResult::default());
        }
        let result = Self::undo(&moves, source_path, &journal.created_dirs, progress);
        // ما أُرجع يُعلَّم غير منقول، وما فشل يبقى معلَّمًا لإعادة المحاولة
        let failed: HashSet<&PathBuf> = result.failed.iter().map(|m| &m.to).collect();
        for entry in journal.entries.iter_mut().filter(|e| e.done) {
            if !failed.contains(&entry.to) {
                entry.done = false;
            }
        }
        let _ = journal.save();
        Some(result)
    }

    /// يرجع (الفنان، العنوان للتجميع، عنوان الوسوم المنظَّف إن كان صالحاً اسماً للملف)
    fn extract_audio_info(file_path: &Path) -> (String, String, Option<String>) {
        let raw_stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("Track");
        let mut artist = "".to_string();
        let mut title = raw_stem.to_string();
        let mut tag_title = None;

        if let Ok(tagged_file) = Probe::open(file_path).and_then(|p| p.read()) {
            if let Some(tag) = tagged_file.primary_tag().or_else(|| tagged_file.first_tag()) {
                if let Some(a) = tag.artist() {
                    let a_trim = fix_mojibake(a.trim());
                    if !a_trim.is_empty() && !a_trim.to_lowercase().contains("unknown") && !a_trim.starts_with("AUD-") && !looks_garbled(&a_trim) {
                        artist = a_trim;
                    }
                }
                if let Some(t) = tag.title() {
                    let t = fix_mojibake(t.trim());
                    // عنوان مشوّه بترميز خاطئ لا يُستخدم أبداً بدل اسم ملف سليم
                    if !t.is_empty() && !looks_garbled(&t) {
                        title = t.clone();
                        tag_title = Some(Self::clean_string(&t));
                    }
                }
            }
        }

        if artist.is_empty() && raw_stem.contains('-') {
            let parts: Vec<&str> = raw_stem.splitn(2, '-').collect();
            if parts.len() == 2 {
                artist = parts[0].trim().to_string();
                title = parts[1].trim().to_string();
            }
        }

        // الفنان يبقى خاماً (وفارغاً إن لم يوجد)؛ يُنظَّف في extract_group_key
        let clean_tit = Self::clean_string(&title);

        (artist, if clean_tit.is_empty() { "Audio Track".to_string() } else { clean_tit }, tag_title)
    }

    /// الاسم الجديد: الأغنى بالمعلومات بين اسم الملف وعنوان الوسوم، حتى لا يضيع جزء من الاسم
    /// ("YouTube (1)" ← عنوان الوسوم الكامل، و"محاضرة - الشيخ - الحلقة 20" يبقى كاملاً)
    fn best_name(stem: &str, tag_title: Option<String>) -> String {
        let from_file = Self::clean_string(stem);
        let letters = |s: &str| s.chars().filter(|c| c.is_alphabetic()).count();
        match tag_title {
            Some(t) if letters(&t) > letters(&from_file) => t,
            _ => from_file,
        }
    }

    /// نوع التسمية المقابل لنوع الملف
    fn naming_kind(file_type: &FileType) -> Kind {
        match file_type {
            FileType::Image => Kind::Image,
            FileType::Video => Kind::Video,
            FileType::Audio => Kind::Audio,
            FileType::Document => Kind::Document,
            FileType::Archive | FileType::Installer | FileType::Other => Kind::Other,
        }
    }

    /// اسم من وسوم الصوت: "الفنان - العنوان"، والفنان يُرفض إن كان مجهولاً أو كوداً
    fn tag_name(artist: &str, title: &str) -> String {
        let title = Self::clean_string(title);
        let artist = Self::clean_string(artist);
        let artist_ok = !artist.is_empty()
            && !artist.to_lowercase().contains("unknown")
            && !naming::is_junk_name(&artist);
        if artist_ok { format!("{artist} - {title}") } else { title }
    }

    /// الاسم الجديد من مصدر موثوق فقط؛ None يعني أن الاسم يبقى كما هو.
    /// لا مصدر ← لا تسمية، وتاريخ آخر تعديل ليس مصدراً.
    fn trusted_name(f: &RawFileInfo, options: &Options) -> Option<String> {
        let group = f.naming_group.filter(|g| options.naming_enabled(*g))?;
        let name = match f.kind {
            Kind::Document => naming::document_title(&f.path)?,
            Kind::Audio => match f.audio_tag_name.as_deref().filter(|t| !naming::is_junk_name(t)) {
                Some(tag_name) => tag_name.to_string(),
                None => {
                    let (date, _) = naming::trusted_date(&f.path, Kind::Audio)?;
                    let word = f.placeholder_context.clone().unwrap_or_else(|| "تسجيل صوتي".to_string());
                    naming::join_name(&date, Some(&word))
                }
            },
            _ => {
                let (date, _) = naming::trusted_date(&f.path, f.kind)?;
                let tag = match group {
                    NamingGroup::Whatsapp => Some("واتساب".to_string()),
                    NamingGroup::Screenshot => Some("لقطة شاشة".to_string()),
                    // أسماء الكاميرا تحتفظ برقمها الأصلي
                    NamingGroup::Camera => f.path.file_stem().map(|s| s.to_string_lossy().to_string()),
                    _ => None,
                };
                naming::join_name(&date, tag.as_deref())
            }
        };
        let clean = match f.kind {
            Kind::Document | Kind::Audio => Self::clean_string(&name),
            _ => sanitize_filename(&name),
        };
        (!clean.is_empty()).then_some(clean)
    }

    /// يرجع (عمليات النقل، عدد الملفات الموجودة أصلاً في مكانها الصحيح، مكان كل ملف بعد التنفيذ،
    /// عدد المكررات التي أُرسلت للمراجعة)
    fn plan_moves(mut files: Vec<(RawFileInfo, PathBuf)>, source_path: &Path) -> (Vec<PlannedMove>, usize, HashMap<PathBuf, PathBuf>, usize) {
        let duplicates = Self::redirect_duplicates(&mut files, source_path);
        // أسماء محجوزة داخل الخطة نفسها (بأحرف صغيرة لأن أسماء ويندوز لا تفرّق بين الحالة)
        let mut reserved: HashSet<String> = HashSet::new();
        let mut moves = Vec::new();
        let mut in_place = 0;
        let mut finals: HashMap<PathBuf, PathBuf> = HashMap::new();
        let lower = |p: &Path| p.to_string_lossy().to_lowercase();

        for (f, target_dir) in files {
            let mut dest_path = target_dir.join(format!("{}.{}", f.new_stem, f.extension));
            let mut counter = 1;

            // تجنب تكرار أسماء الملفات المتشابهة (الموجودة فعلاً أو المحجوزة في الخطة)؛
            // إذا كانت الوجهة هي مكان الملف نفسه فهو في مكانه الصحيح ولا يتحرك
            let source_key = lower(&f.path);
            while lower(&dest_path) != source_key && (dest_path.exists() || reserved.contains(&lower(&dest_path))) {
                dest_path = target_dir.join(format!("{} ({}).{}", f.new_stem, counter, f.extension));
                counter += 1;
            }
            reserved.insert(lower(&dest_path));

            if lower(&dest_path) == source_key {
                in_place += 1;
            } else {
                moves.push(PlannedMove { from: f.path.clone(), to: dest_path.clone() });
            }
            finals.insert(f.path, dest_path);
        }

        (moves, in_place, finals, duplicates)
    }

    /// المكررات: تجميع بالحجم أولاً، ثم بصمة المحتوى للأحجام المتساوية فقط،
    /// ثم داخل كل مجموعة محتوى واحدة يُختار الأصل: الأقدم تاريخًا، وعند التساوي
    /// الاسم الأوضح (غير الرديء وبلا لاحقة «(1)»). الباقي إلى «مكررات للمراجعة». لا حذف.
    fn redirect_duplicates(files: &mut [(RawFileInfo, PathBuf)], source_path: &Path) -> usize {
        let mut by_size: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, (f, _)) in files.iter().enumerate() {
            // الملف الفارغ ليس نسخة مكررة: كل الملفات الفارغة متطابقة بلا معنى
            if let Ok(meta) = fs::metadata(&f.path) {
                if meta.is_file() && meta.len() > 0 {
                    by_size.entry(meta.len()).or_default().push(i);
                }
            }
        }
        let review_dir = source_path.join(DUPLICATES_FOLDER);
        let mut count = 0;
        for indices in by_size.values().filter(|v| v.len() > 1) {
            let mut by_digest: HashMap<u64, Vec<usize>> = HashMap::new();
            for &i in indices {
                if let Some(digest) = Self::digest(&files[i].0.path) {
                    by_digest.entry(digest).or_default().push(i);
                }
            }
            for group in by_digest.values().filter(|v| v.len() > 1) {
                for i in Self::rank_copies(files, group) {
                    files[i].1 = review_dir.clone();
                    count += 1;
                }
            }
        }
        count
    }

    /// ترتيب نسخ متطابقة المحتوى: يرجّع النسخ التي تُنقل للمراجعة (كل ما عدا الأصل)
    fn rank_copies(files: &[(RawFileInfo, PathBuf)], group: &[usize]) -> Vec<usize> {
        let mut ranked: Vec<(u64, bool, bool, String, usize)> = group
            .iter()
            .map(|&i| {
                let path = &files[i].0.path;
                let stamp = fs::metadata(path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(u64::MAX);
                let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                let vague = naming::is_junk_name(&stem);
                let numbered = stem.contains("(1)") || stem.contains("(2)");
                (stamp, vague, numbered, stem, i)
            })
            .collect();
        ranked.sort();
        ranked.into_iter().skip(1).map(|(_, _, _, _, i)| i).collect()
    }

    /// بصمة محتوى الملف (SipHash-1-3، 64 بت). تُحسب للأحجام المتساوية فقط.
    /// ponytail: تصادم نظري 1 من 2^64؛ لو احتجت يقينًا بايتًا ببايت فقارن الملفين مباشرة.
    fn digest(path: &Path) -> Option<u64> {
        use std::hash::Hasher;
        use std::io::Read;
        let mut file = fs::File::open(path).ok()?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = file.read(&mut buf).ok()?;
            if n == 0 {
                return Some(hasher.finish());
            }
            hasher.write(&buf[..n]);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// وضع إعادة البناء: تُفرَّغ المجلدات الفرعية ويُعاد تنظيمها
    fn rebuild() -> Options {
        Options { keep_existing_folders: false, ..Options::default() }
    }

    fn keywords() -> Vec<String> {
        DEFAULT_SPEAKER_KEYWORDS.iter().map(|k| OrganizerEngine::group_key(k)).collect()
    }

    fn group(artist: &str, title: &str) -> (String, String) {
        OrganizerEngine::extract_group_key(artist, title, &keywords())
    }

    fn group_of(file_name: &str) -> String {
        let (artist, title, _) = OrganizerEngine::extract_audio_info(Path::new(file_name));
        group(&artist, &title).0
    }

    #[test]
    fn artist_from_file_name() {
        assert_eq!(group_of("عمرو دياب - تملي معاك.mp3"), "عمرو دياب");
    }

    #[test]
    fn no_artist_goes_to_single_tracks() {
        assert_eq!(group_of("track01.mp3"), "Single Tracks");
    }

    #[test]
    fn episode_with_speaker_in_file_name() {
        assert_eq!(group_of("الحلقة 19 - الشيخ محمد.mp3"), "الشيخ محمد");
        assert_eq!(group_of("الحلقة 20 - الشيخ محمد.mp3"), "الشيخ محمد");
    }

    #[test]
    fn episode_without_speaker_is_a_series() {
        assert_eq!(group_of("الحلقة 19.mp3"), "سلسلة الحلقات");
    }

    #[test]
    fn known_speaker_suffix_matches_after_normalization() {
        assert_eq!(group("", "محاضرة - أدهم الشرقاوي").0, "أدهم الشرقاوي");
    }

    #[test]
    fn speaker_keywords_come_from_options() {
        let custom = vec![OrganizerEngine::group_key("القارئ")];
        let (name, _) = OrganizerEngine::extract_group_key("", "سورة الكهف - القارئ ماهر", &custom);
        assert_eq!(name, "القارئ ماهر");
        // بدون الكلمة في القائمة لا يُعرف صاحب المقطع
        assert_eq!(group("", "سورة الكهف - القارئ ماهر").0, "Single Tracks");
    }

    #[test]
    fn artist_key_ignores_case_and_hamza() {
        assert_eq!(group("Amr Diab", "x").1, group("amr diab", "x").1);
        assert_eq!(group("أحمد", "x").1, group("احمد", "x").1);
    }

    #[test]
    fn short_arabic_artist_counted_in_letters() {
        // "علي" ثلاثة أحرف (6 بايت) فهو اسم صالح، و"عل" حرفان فقط
        assert_eq!(group("علي", "x").0, "علي");
        assert_eq!(group("عل", "x").0, "Single Tracks");
    }

    /// مجلد مؤقت فيه ملفات وهمية فارغة — لا يلمس أي بيانات حقيقية
    pub(crate) fn temp_tree(name: &str, files: &[&str]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("array_test_{}_{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        for f in files {
            let p = root.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, b"").unwrap();
        }
        root
    }

    /// صورة JPEG صغيرة فيها EXIF: DateTimeOriginal داخل Exif IFD كما في الصور الحقيقية
    pub(crate) fn jpeg_with_date(path: &Path, date: &str) {
        let mut tiff = Vec::new();
        tiff.extend(b"II*\x00");
        tiff.extend(8u32.to_le_bytes()); // IFD0 عند البايت 8
        // IFD0: مؤشر واحد إلى Exif IFD (0x8769) عند البايت 26
        tiff.extend(1u16.to_le_bytes());
        tiff.extend(0x8769u16.to_le_bytes());
        tiff.extend(4u16.to_le_bytes());
        tiff.extend(1u32.to_le_bytes());
        tiff.extend(26u32.to_le_bytes());
        tiff.extend(0u32.to_le_bytes());
        // Exif IFD: DateTimeOriginal (0x9003) والنص عند البايت 44
        tiff.extend(1u16.to_le_bytes());
        tiff.extend(0x9003u16.to_le_bytes());
        tiff.extend(2u16.to_le_bytes());
        tiff.extend(20u32.to_le_bytes());
        tiff.extend(44u32.to_le_bytes());
        tiff.extend(0u32.to_le_bytes());
        let mut text = format!("{date}\x00").into_bytes();
        text.resize(20, 0);
        tiff.extend(text);
        let mut app1 = b"Exif\x00\x00".to_vec();
        app1.extend(tiff);
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1];
        jpeg.extend(((app1.len() + 2) as u16).to_be_bytes());
        jpeg.extend(app1);
        jpeg.extend([0xFF, 0xD9]);
        fs::write(path, jpeg).unwrap();
    }

    /// ملف PDF أدنى فيه قاموس /Title
    pub(crate) fn pdf_with_title(path: &Path, title: &str) {
        fs::write(
            path,
            format!("%PDF-1.4
1 0 obj
<< /Title ({title}) >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
"),
        )
        .unwrap();
    }

    fn organize(root: &Path, options: &Options) -> (Plan, RunResult) {
        let plan = OrganizerEngine::plan(root, options).unwrap();
        let progress = AtomicUsize::new(0);
        let result = OrganizerEngine::execute(&plan, &progress);
        assert_eq!(progress.load(Ordering::Relaxed), plan.moves.len());
        (plan, result)
    }

    fn undo(plan: &Plan, moves: &[PlannedMove]) -> RunResult {
        OrganizerEngine::undo(moves, &plan.source, &plan.new_dirs, &AtomicUsize::new(0))
    }

    /// كل الملفات تحت المجلد (مسارات نسبية بشرطة /) مرتبة، للمقارنة قبل وبعد
    /// شجرة الملفات للاختبارات، بلا مجلد البيان `.array`
    fn tree(root: &Path) -> Vec<String> {
        let mut files: Vec<String> = WalkDir::new(root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.path().strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
        files.retain(|f| !f.starts_with(".array/"));
        files.sort();
        files
    }

    #[test]
    fn plan_moves_nothing() {
        let root = temp_tree("plan", &["a/IMG_1.jpg", "IMG 1.jpg"]);
        let plan = OrganizerEngine::plan(&root, &rebuild()).unwrap();

        assert_eq!(plan.moves.len(), 2);
        assert!(root.join("a/IMG_1.jpg").exists() && !root.join("Pictures").exists());
        // اسمان يصبحان متطابقين بعد التنظيف يأخذان اسمين مختلفين داخل الخطة
        assert_ne!(plan.moves[0].to, plan.moves[1].to);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn videos_go_to_their_year_and_live_photos_stay_with_their_photo() {
        let root = temp_tree(
            "videos",
            &["20190704_101010.mp4", "VID-20210101-WA0001.mp4", "IMG_0548.HEIC", "IMG_0548.MOV", "trip.mp4", "Videos/فيديو 2019/old.mp4"],
        );
        let (_, result) = organize(&root, &Options::default());
        assert!(result.failed.is_empty());
        let files = tree(&root);
        // مجلد السنة الموجود ("فيديو 2019") يُستخدم بدل إنشاء "2019"
        assert!(files.contains(&"Videos/فيديو 2019/2019-07-04 10-10.mp4".to_string()), "{files:?}");
        assert!(files.contains(&"Videos/2021/2021-01-01 واتساب.mp4".to_string()), "{files:?}");
        // الصورة الحية: فيديو .mov يُنقل مع صورته .heic كحزمة واحدة بنفس الاسم
        assert!(files.contains(&"صور/2026/IMG 0548.heic".to_string()), "{files:?}");
        assert!(files.contains(&"صور/2026/IMG 0548.mov".to_string()), "{files:?}");
        // بلا تاريخ في الاسم: تاريخ التعديل يختار المجلد فقط
        assert!(files.contains(&"Videos/2026/trip.mp4".to_string()), "{files:?}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn videos_by_year_can_be_turned_off() {
        let root = temp_tree("videos_off", &["20190704_101010.mp4"]);
        let (_, result) = organize(&root, &Options { videos_by_year: false, ..Options::default() });
        assert!(result.failed.is_empty());
        assert_eq!(tree(&root), ["فيديو/2019-07-04 10-10.mp4"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn empties_subfolders_and_organizes_inside_the_selected_folder() {
        // المثال: مجلد "صور" موجود يُعاد استخدامه، والفئات الأخرى تُنشأ، و"رحلة 2023" يُفرَّغ ويُحذف
        let root = temp_tree("inplace", &["رحلة 2023/sea.jpg", "رحلة 2023/clip.mp4", "صور/cat.png", "cv.pdf"]);

        let (plan, result) = organize(&root, &rebuild());

        assert!(result.failed.is_empty());
        assert_eq!(
            tree(&root),
            ["صور/2026/cat.png", "صور/2026/sea.jpg", "فيديو/2026/clip.mp4", "مستندات/PDF/cv.pdf"]
        );
        assert!(!root.join("رحلة 2023").exists());
        assert_eq!(
            plan.new_dirs,
            [
                root.join("صور/2026"),
                root.join("فيديو"),
                root.join("فيديو/2026"),
                root.join("مستندات"),
                root.join("مستندات/PDF"),
            ]
        );
        // كل ملف دخل مجلد سنته: لا شيء في مكانه الأصلي
        assert_eq!(plan.in_place, 0);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reuses_existing_folders_by_arabic_or_english_name() {
        let root = temp_tree("reuse", &[
            "Photos/old.jpg",
            "الموسيقى/old.mp3",
            "مستندات/old.pdf",
            "new/sea.jpg",
            "new/song.mp3",
            "new/cv.docx",
        ]);

        organize(&root, &rebuild());

        assert!(root.join("Photos/2026/sea.jpg").exists());
        assert!(root.join("الموسيقى/منوعات/song.mp3").exists());
        assert!(root.join("مستندات/Word/cv.docx").exists());
        // لم تُنشأ مجلدات فئات جديدة بالاسم الافتراضي: المجلدات الموجودة كافية
        assert!(!root.join("صور").exists() && !root.join("صوتيات").exists() && !root.join("Pictures").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn running_twice_changes_nothing() {
        let root = temp_tree("twice", &[
            "a/الحلقة 1 - الشيخ محمد.mp3",
            "a/الحلقة 2 - الشيخ محمد.mp3",
            "b/الحلقة 3 - الشيخ محمد.mp3",
            "IMG_001.jpg",
            "docs/report (final).pdf",
        ]);
        organize(&root, &rebuild());
        let after_first = tree(&root);

        let second = OrganizerEngine::plan(&root, &rebuild()).unwrap();

        assert!(second.moves.is_empty(), "{:?}", second.moves);
        assert_eq!(second.in_place, 5);
        assert_eq!(tree(&root), after_first);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reuses_an_existing_artist_folder() {
        let root = temp_tree("artist", &[
            "Music/عمرو دياب/old.mp3",
            "x/عمرو دياب - أغنية 1.mp3",
            "x/عمرو دياب - أغنية 2.mp3",
            "x/عمرو دياب - أغنية 3.mp3",
        ]);

        organize(&root, &rebuild());

        assert_eq!(fs::read_dir(root.join("Music/عمرو دياب")).unwrap().count(), 4);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn never_writes_into_a_program_folder_named_like_a_category() {
        let root = temp_tree("cat_program", &["Music/player.exe", "Music/skin.png", "new/song.mp3"]);

        organize(&root, &rebuild());

        assert!(root.join("صوتيات/منوعات/song.mp3").exists());
        assert_eq!(fs::read_dir(root.join("Music")).unwrap().count(), 2);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn organizes_a_folder_end_to_end() {
        let root = temp_tree("e2e", &[
            "a/الحلقة 1 - الشيخ محمد.mp3",
            "a/الحلقة 2 - الشيخ محمد.mp3",
            "b/الحلقة 3 - الشيخ محمد.mp3",
            "song_one.mp3",
            "IMG_001.jpg",
            "work/report (final).pdf",
            "repo/.git/notes.txt",
            "repo/readme.txt",
            "AppData/config.txt",
        ]);
        fs::create_dir_all(root.join("repo/.git/empty")).unwrap();
        fs::create_dir_all(root.join("keep_me/inner")).unwrap();

        let (_, result) = organize(&root, &rebuild());

        assert_eq!(result.done.len(), 6);
        assert!(result.failed.is_empty());
        // رقم الحلقة يبقى في الاسم
        assert!(root.join("صوتيات/الشيخ محمد/الحلقة 1 - الشيخ محمد.mp3").exists(), "{:?}", tree(&root));
        assert!(root.join("صوتيات/الشيخ محمد/الحلقة 2 - الشيخ محمد.mp3").exists());
        assert!(root.join("صوتيات/الشيخ محمد/الحلقة 3 - الشيخ محمد.mp3").exists());
        assert!(root.join("صوتيات/منوعات/song one.mp3").exists());
        assert!(root.join("صور/2026/IMG 001.jpg").exists());
        // الأقواس ذات المعنى تبقى في الاسم، والمستند تحت مجلد نوعه
        assert!(root.join("مستندات/PDF/report (final).pdf").exists());
        // المجلدات المخفية ومجلدات البرامج لا تُلمس
        assert!(root.join("repo/.git/notes.txt").exists());
        assert!(root.join("repo/readme.txt").exists());
        assert!(root.join("repo/.git/empty").is_dir());
        assert!(root.join("AppData/config.txt").exists());
        // المجلدات التي أُفرغت تُحذف، والتي كانت فارغة قبل التشغيل تبقى
        assert!(!root.join("a").exists() && !root.join("work").exists());
        assert!(root.join("keep_me/inner").is_dir());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn never_touches_programs_projects_scripts_or_web_pages() {
        let root = temp_tree("programs", &[
            "Photos/sea.jpg",
            "MyTool/tool.exe",
            "MyTool/settings.txt",
            "MyTool/icon.png",
            "MyProject/Cargo.toml",
            "MyProject/assets/logo.png",
            "MyProject/notes.txt",
            "Scripts/backup.ps1",
            "Scripts/list.txt",
            "SavedPage/page.html",
            "SavedPage/page_files/img.jpg",
            "Game/config.ini",
            "Game/screenshot.png",
            "Lib/core.dll",
            "Lib/manual.pdf",
            "loose.jpg",
        ]);

        let plan = OrganizerEngine::plan(&root, &rebuild()).unwrap();

        // Photos مجلد صور موجود: كل صورة تنتقل إلى مجلد سنتها داخله (التعديل يختار المجلد فقط)
        let mut moved: Vec<_> = plan.moves.iter().map(|m| m.from.strip_prefix(&root).unwrap().to_path_buf()).collect();
        moved.sort();
        assert_eq!(moved, vec![PathBuf::from("Photos\\sea.jpg"), PathBuf::from("loose.jpg")]);
        let loose = plan.moves.iter().find(|m| m.from == root.join("loose.jpg")).unwrap();
        assert_eq!(loose.to, root.join("Photos").join("2026").join("loose.jpg"));
        let mut skipped: Vec<_> = plan.skipped.iter().map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        skipped.sort();
        assert_eq!(skipped, ["Game", "Lib", "MyProject", "MyTool", "SavedPage", "Scripts"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_installer_alone_does_not_make_a_folder_a_program() {
        // مجلد فيه مكتبة برنامج: يبقى مرفوضاً كما كان
        let root = temp_tree("tool_root", &["App/tool.exe", "App/core.dll", "Download/setup.exe", "Download/photo.jpg"]);
        let err = OrganizerEngine::plan(&root.join("App"), &rebuild()).err().unwrap();
        assert!(err.contains("program"), "{err}");
        // مجلد تنزيلات فيه مثبّت واحد: يُرتَّب والمثبّت يذهب إلى «مثبّتات» (بند 17)
        let plan = OrganizerEngine::plan(&root.join("Download"), &rebuild()).unwrap();
        assert!(plan.moves.iter().any(|m| m.to.ends_with("مثبّتات/setup.exe")), "{:?}", plan.moves);
        assert!(root.join("Download/photo.jpg").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn handles_all_media_and_document_types() {
        let root = temp_tree("types", &["a.heic", "b.mov", "c.flac", "d.xls", "e.7z", "f.lnk", "g.db", "h.unknown"]);
        let plan = OrganizerEngine::plan(&root, &rebuild()).unwrap();

        let mut targets: Vec<_> = plan
            .moves
            .iter()
            .map(|m| m.to.strip_prefix(&root).unwrap().components().next().unwrap().as_os_str().to_string_lossy().to_string())
            .collect();
        targets.sort();
        assert_eq!(targets, ["تسجيلات", "صور", "فيديو", "مستندات", "مضغوطات"]);
        // اختصارات وقواعد بيانات وملفات غير معروفة لا تُلمس
        assert_eq!(plan.moves.len(), 5);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn meaningless_names_stay_without_a_trusted_source() {
        // المرحلة 2: بلا EXIF وبلا تاريخ في الاسم لا تسمية (تاريخ التعديل مرفوض)
        // مثل D:\Music عند المستخدم: المجلد المختار هو مجلد الموسيقى نفسه
        let root = temp_tree("junk", &[
            "Music/house lo [AUD/4749d951].mp3",
            "Music/house lo [AUD/0750707c] (1).mp3",
            "Music/Single Tracks/14c58cdd].mp3",
            "Music/Single Tracks/YouTube (1).mp4",
            "Music/Single Tracks/Ma Yehtaj.mp3",
        ]);
        let music = root.join("Music");

        organize(&music, &rebuild());

        let files = tree(&music);
        // الأسماء الرديئة تبقى كما هي: لا مصدر موثوق لاسم جديد
        assert!(files.contains(&"تسجيلات/2026/4749d951].mp3".to_string()), "{files:?}");
        assert!(files.contains(&"تسجيلات/2026/0750707c].mp3".to_string()), "{files:?}");
        assert!(files.contains(&"تسجيلات/2026/14c58cdd].mp3".to_string()), "{files:?}");
        // ولا تاريخ تعديل يُضاف إلى أي اسم
        assert!(!files.iter().any(|f| f.contains(" - 20")), "{files:?}");
        // الاسم المفهوم لا يتغير
        assert!(files.contains(&"منوعات/Ma Yehtaj.mp3".to_string()), "{files:?}");
        assert!(files.contains(&"فيديو/2026/YouTube.mp4".to_string()), "{files:?}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn installers_are_moved_from_the_root_only() {
        let root = temp_tree("installers", &["setup.exe", "setup.msi", "tools/helper.exe", "photo.jpg"]);
        organize(&root, &rebuild());
        let files = tree(&root);
        assert!(files.contains(&"مثبّتات/setup.exe".to_string()), "{files:?}");
        assert!(files.contains(&"مثبّتات/setup.msi".to_string()), "{files:?}");
        // مثبّت داخل مجلد فرعي: مجلد البرنامج يبقى كما هو
        assert!(files.contains(&"tools/helper.exe".to_string()), "{files:?}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_package_moves_and_is_named_with_its_master() {
        let root = temp_tree("package", &["VID_20240102_101010.mp4", "VID_20240102_101010.srt", "photo.jpg", "photo.xmp", "lone.srt"]);
        organize(&root, &rebuild());
        let files = tree(&root);
        let video = files.iter().find(|f| f.ends_with(".mp4")).expect("missing video").clone();
        let subtitle = files.iter().find(|f| f.ends_with(".srt") && f.contains("2024")).expect("missing subtitle").clone();
        // الترجمة مع الفيديو في نفس المجلد وبنفس الاسم الجديد
        assert_eq!(video.trim_end_matches(".mp4"), subtitle.trim_end_matches(".srt"));
        assert!(video.starts_with("فيديو/2024/"), "{files:?}");
        // بيانات التعديل تتبع صورتها
        let photo = files.iter().find(|f| f.ends_with(".jpg")).expect("missing photo").clone();
        let sidecar = files.iter().find(|f| f.ends_with(".xmp")).expect("missing xmp").clone();
        assert_eq!(photo.trim_end_matches(".jpg"), sidecar.trim_end_matches(".xmp"));
        // ترجمة بلا فيديو تبقى مكانها ولا تُلمس
        assert!(files.contains(&"lone.srt".to_string()), "{files:?}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_year_splits_into_months_only_after_500_files() {
        let root = temp_tree("year_split", &[]);
        let photos = root.join("صور");
        fs::create_dir_all(&photos).unwrap();
        let make = |from: usize, to: usize| {
            for i in from..to {
                fs::write(photos.join(format!("IMG_20240102_{i:04}.jpg")), b"").unwrap();
            }
        };
        let targets = |plan: &Plan| -> Vec<String> {
            plan.moves.iter().map(|m| m.to.to_string_lossy().replace('\\', "/")).collect()
        };

        make(0, 500);
        let plan = OrganizerEngine::plan(&root, &rebuild()).unwrap();
        assert_eq!(plan.moves.len(), 500);
        assert!(targets(&plan).iter().all(|t| !t.contains("صور/2024/01/")), "قُسمت السنة عند 500");

        // الملف 501 هو بداية التقسيم إلى أشهر
        make(500, 501);
        let plan = OrganizerEngine::plan(&root, &rebuild()).unwrap();
        assert_eq!(plan.moves.len(), 501);
        assert!(targets(&plan).iter().all(|t| t.contains("صور/2024/01/")), "{:?}", targets(&plan).first());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn colliding_names_never_overwrite_each_other() {
        let root = temp_tree("collide", &[]);
        fs::write(root.join("photo.jpg"), b"first").unwrap();
        fs::write(root.join("photo (1).jpg"), b"second").unwrap();
        organize(&root, &rebuild());
        let mut contents: Vec<String> = tree(&root)
            .iter()
            .map(|f| fs::read_to_string(root.join(f)).unwrap_or_default())
            .filter(|c| !c.is_empty())
            .collect();
        contents.sort();
        // لا يضيع محتوى: لا كتابة فوق أي ملف (بند 21 من التشخيص)
        assert_eq!(contents, ["first", "second"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_file_with_only_a_modified_date_is_not_renamed() {
        let root = temp_tree("modified_only", &["4749d951.jpg", "scan1.png"]);
        jpeg_with_date(&root.join("4749d951.jpg"), "2024:05:12 14:30:05");
        // scan1.png ملف فارغ: له تاريخ تعديل فقط
        organize(&root, &rebuild());
        let files = tree(&root);
        assert!(files.contains(&"صور/2026/scan1.png".to_string()), "{files:?}");
        // الصورة التي لها EXIF تُسمّى وتُوضع في سنة التصوير
        assert!(files.contains(&"صور/2024/2024-05-12 14-30-05.jpg".to_string()), "{files:?}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_template_title_is_not_used_as_a_name() {
        let root = temp_tree("template_title", &[]);
        pdf_with_title(&root.join("4749d951.pdf"), "Microsoft Word - Document1");
        organize(&root, &rebuild());
        assert!(tree(&root).contains(&"مستندات/PDF/4749d951.pdf".to_string()), "{:?}", tree(&root));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_document_gets_its_internal_title() {
        let root = temp_tree("doc_title", &[]);
        pdf_with_title(&root.join("4749d951.pdf"), "Annual Report 2024");
        organize(&root, &rebuild());
        assert!(tree(&root).contains(&"مستندات/PDF/Annual Report 2024.pdf".to_string()), "{:?}", tree(&root));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn camera_names_keep_their_original_number() {
        let root = temp_tree("camera", &[]);
        jpeg_with_date(&root.join("IMG_2353.jpg"), "2024:05:12 14:30:05");
        organize(&root, &rebuild());
        assert!(
            tree(&root).contains(&"صور/2024/2024-05-12 14-30-05 IMG_2353.jpg".to_string()),
            "{:?}",
            tree(&root)
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn whatsapp_and_screenshot_files_get_their_label() {
        let root = temp_tree("labels", &["IMG-20240512-WA0003.jpg", "Screenshot 2024-05-12 143005.png"]);
        organize(&root, &rebuild());
        let files = tree(&root);
        // اسم واتساب يحمل التاريخ بلا وقت، واللقطة تحمل الوقت
        assert!(files.contains(&"صور/2024/2024-05-12 واتساب.jpg".to_string()), "{files:?}");
        assert!(files.contains(&"صور/2024/2024-05-12 14-30 لقطة شاشة.png".to_string()), "{files:?}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn ten_photos_in_the_same_second_get_ten_names() {
        let root = temp_tree("same_second", &[]);
        for i in 1..=10 {
            jpeg_with_date(&root.join(format!("scan{i}.jpg")), "2024:05:12 14:30:05");
        }
        let plan = OrganizerEngine::plan(&root, &rebuild()).unwrap();
        assert_eq!(plan.moves.len(), 10);
        let targets: HashSet<String> = plan.moves.iter().map(|m| m.to.to_string_lossy().to_lowercase()).collect();
        assert_eq!(targets.len(), 10, "أسماء مكررة: {targets:?}");
        assert!(plan.moves.iter().all(|m| !m.to.exists() || m.to == m.from));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_disabled_group_keeps_its_files_unnamed() {
        let root = temp_tree("group_off", &["IMG-20240512-WA0003.jpg", "scan1.jpg"]);
        jpeg_with_date(&root.join("scan1.jpg"), "2024:05:12 14:30:05");
        organize(&root, &Options { rename_whatsapp_media: false, ..rebuild() });
        let files = tree(&root);
        assert!(files.contains(&"صور/2024/IMG-20240512-WA0003.jpg".to_string()), "{files:?}");
        assert!(files.contains(&"صور/2024/2024-05-12 14-30-05.jpg".to_string()), "{files:?}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn meaningless_names_stay_when_renaming_is_off() {
        let root = temp_tree("junk_off", &["x/4749d951].mp3"]);
        organize(&root, &Options { rename_files: false, ..rebuild() });
        assert_eq!(tree(&root), ["تسجيلات/2026/4749d951].mp3"]);
        fs::remove_dir_all(&root).unwrap();
    }

    /// مثل D:\Photo عند المستخدم: مجلدات أشخاص وسنوات منظمة، وملفات متفرقة في المجلد نفسه
    #[test]
    fn default_mode_never_touches_existing_folders() {
        let organized = [
            "Photo/Ali/IMG_0001.jpg",
            "Photo/Mom/133649_466985853345771_200500525.jpg",
            "Photo/الصور من 2016/clip.mp4",
            "Photo/الصور من 2016/IMG_2016.HEIC",
            "Photo/بلا عنوان/4749d951.png",
        ];
        let loose = ["Photo/IMG_0612.jpg", "Photo/133649_466985853345771_200500525.jpg", "Photo/trip.mp4", "Photo/cv.pdf"];
        let root = temp_tree("keep", &[&organized[..], &loose[..]].concat());
        let photo = root.join("Photo");

        let (plan, result) = organize(&photo, &Options::default());

        assert_eq!(plan.kept_in_folders, organized.len());
        assert!(result.failed.is_empty());
        // كل ملف داخل المجلدات المنظمة باقٍ بنفس الاسم والمكان
        for f in organized {
            assert!(root.join(f).exists(), "{f} was touched");
        }
        // المجلد المختار اسمه Photo فهو مجلد الصور نفسه: الصور المتفرقة تبقى فيه، والباقي لمجلده
        let files = tree(&photo);
        assert!(files.contains(&"2026/IMG 0612.jpg".to_string()), "{files:?}");
        // بلا مصدر موثوق يبقى الاسم الرديء كما هو
        assert!(files.contains(&"2026/133649 466985853345771 200500525.jpg".to_string()), "{files:?}");
        assert!(files.contains(&"فيديو/2026/trip.mp4".to_string()), "{files:?}");
        assert!(files.contains(&"مستندات/PDF/cv.pdf".to_string()), "{files:?}");
        assert!(!photo.join("Pictures").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn default_mode_still_files_loose_music_into_an_existing_artist_folder() {
        let root = temp_tree("keep_music", &[
            "Music/القارئ عبدالله الموسى/عبدالله الموسى 1.mp3",
            "Music/القارئ عبدالله الموسى/عبدالله الموسى 2.mp3",
            "Music/قناة القارئ عبدالله الموسى/تلاوة للقارئ عبدالله الموسى.mp3",
            "Music/سورة مريم بصوت الشيخ عبدالله الموسى.mp3",
        ]);
        let music = root.join("Music");

        organize(&music, &Options::default());

        let files = tree(&music);
        // الملف المتفرق يذهب للمجلد الموجود، والمجلدات الموجودة لا تُفرَّغ في هذا الوضع
        assert!(files.contains(&"القارئ عبدالله الموسى/سورة مريم بصوت الشيخ عبدالله الموسى.mp3".to_string()), "{files:?}");
        assert!(files.contains(&"قناة القارئ عبدالله الموسى/تلاوة للقارئ عبدالله الموسى.mp3".to_string()), "{files:?}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn default_mode_empties_folders_left_by_old_versions() {
        let root = temp_tree("keep_old", &["Music/Single Tracks/song.mp3", "Music/Unknown/a b.mp3", "Music/Array_Organized/Pictures/sea.jpg"]);
        let music = root.join("Music");

        organize(&music, &Options::default());

        let files = tree(&music);
        assert!(!music.join("Single Tracks").exists() && !music.join("Unknown").exists() && !music.join("Array_Organized").exists(), "{files:?}");
        assert!(files.contains(&"منوعات/song.mp3".to_string()), "{files:?}");
        assert!(files.contains(&"صور/2026/sea.jpg".to_string()), "{files:?}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn keeps_original_names_when_renaming_is_off() {
        let root = temp_tree("norename", &["work/report (final).PDF", "IMG_001.jpg"]);
        let options = Options { rename_files: false, ..rebuild() };

        organize(&root, &options);

        assert!(root.join("مستندات/PDF/report (final).PDF").exists());
        assert!(root.join("صور/2026/IMG_001.jpg").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn undo_restores_the_exact_original_tree() {
        let files = ["رحلة 2023/a.jpg", "رحلة 2023/b.mp4", "a/b/الحلقة 1 - الشيخ محمد.mp3", "report.pdf"];
        let root = temp_tree("undo", &files);
        // مجلد صور فارغ أنشأه المستخدم ويُعاد استخدامه: يجب أن يبقى بعد التراجع
        fs::create_dir_all(root.join("صور")).unwrap();
        fs::create_dir_all(root.join("keep_me")).unwrap();
        let before = tree(&root);

        let (plan, result) = organize(&root, &rebuild());
        assert!(!root.join("رحلة 2023").exists());

        let undone = undo(&plan, &result.done);

        assert_eq!(undone.done.len(), 4);
        assert!(undone.failed.is_empty());
        assert_eq!(tree(&root), before);
        // المجلدات التي أنشأها التنظيم حُذفت، والأصلية باقية حتى الفارغة منها
        assert!(!root.join("Videos").exists() && !root.join("Documents").exists() && !root.join("Music").exists());
        assert!(root.join("صور").is_dir() && root.join("keep_me").is_dir());
        fs::remove_dir_all(&root).unwrap();
    }

    /// تاريخ تعديل ثابت للملف (لاختبار «الأقدم يبقى»)
    fn set_modified(path: &Path, secs: u64) {
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs)).unwrap();
    }

    #[test]
    fn duplicates_are_judged_by_content_not_by_name() {
        let root = temp_tree("dups_content", &[]);
        // اسم شبه متطابق ومحتوى مختلف: ليسا مكررين
        fs::write(root.join("photo.jpg"), b"first-content").unwrap();
        fs::write(root.join("photo (1).jpg"), b"second-cont").unwrap();
        // اسمان مختلفان ومحتوى واحد: مكرران
        fs::write(root.join("أ.txt"), b"identical").unwrap();
        fs::write(root.join("ب.txt"), b"identical").unwrap();

        let (plan, _) = organize(&root, &rebuild());

        assert_eq!(plan.duplicates, 1);
        let files = tree(&root);
        let review: Vec<_> = files.iter().filter(|f| f.starts_with("مكررات للمراجعة/")).collect();
        assert_eq!(review.len(), 1, "{files:?}");
        // الملفان المختلفان بقيا كلاهما
        assert_eq!(files.iter().filter(|f| f.contains("photo")).count(), 2, "{files:?}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn three_copies_keep_the_clearest_oldest_name_and_review_the_rest() {
        let root = temp_tree("dups_three", &[]);
        for name in ["4749d951.txt", "عقد الإيجار (1).txt", "عقد الإيجار.txt"] {
            fs::write(root.join(name), b"same bytes").unwrap();
            set_modified(&root.join(name), 1_700_000_000);
        }

        let (plan, result) = organize(&root, &rebuild());
        assert!(result.failed.is_empty());
        assert_eq!(plan.duplicates, 2);

        let files = tree(&root);
        // الاسم الأوضح (بلا كود وبلا (1)) يبقى في مكانه الطبيعي
        assert!(files.contains(&"مستندات/نصوص/عقد الإيجار.txt".to_string()), "{files:?}");
        assert_eq!(files.iter().filter(|f| f.starts_with("مكررات للمراجعة/")).count(), 2, "{files:?}");

        // التراجع يعيد النسخ الثلاث كلها
        let undone = OrganizerEngine::undo_from_journal(&root, &AtomicUsize::new(0)).unwrap();
        assert!(undone.failed.is_empty());
        let files = tree(&root);
        for name in ["4749d951.txt", "عقد الإيجار (1).txt", "عقد الإيجار.txt"] {
            assert!(files.contains(&name.to_string()), "{name} lost: {files:?}");
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_same_size_stranger_does_not_hide_duplicates() {
        // ملف بنفس حجم نسختين متطابقتين لكن محتواه مختلف: يجب ألا يكسر الكشف
        let root = temp_tree("dups_mixed", &[]);
        fs::write(root.join("photo.jpg"), b"notsame123").unwrap();
        fs::write(root.join("a.txt"), b"identical1").unwrap();
        fs::write(root.join("b.txt"), b"identical1").unwrap();
        let (plan, _) = organize(&root, &rebuild());
        assert_eq!(plan.duplicates, 1);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_journal_restores_a_run_that_stopped_halfway() {
        let root = temp_tree("journal_partial", &["a/1.pdf", "a/2.pdf", "a/3.pdf"]);
        let before = tree(&root);
        let plan = OrganizerEngine::plan(&root, &rebuild()).unwrap();
        // البيان يُكتب قبل أي عملية، ثم نُفِّذت الأولى فقط كما لو انقطع البرنامج
        let mut journal = Journal::new(&plan.source, &plan.moves, &plan.new_dirs);
        journal.save().unwrap();
        let first = &plan.moves[0];
        fs::create_dir_all(first.to.parent().unwrap()).unwrap();
        fs::rename(&first.from, &first.to).unwrap();
        journal.entries[0].done = true;
        journal.save().unwrap();

        let undone = OrganizerEngine::undo_from_journal(&root, &AtomicUsize::new(0)).unwrap();

        assert_eq!(undone.done.len(), 1);
        assert!(undone.failed.is_empty());
        assert_eq!(tree(&root), before);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn undo_reads_the_folder_journal_without_any_appdata_log() {
        let root = temp_tree("journal_full", &["1.pdf", "2.pdf", "clip.mp4", "sea.jpg"]);
        let before = tree(&root);
        let (plan, _) = organize(&root, &rebuild());
        assert!(Journal::exists(&root), "البيان لم يُكتب داخل المجلد");

        // لا سجل AppData هنا إطلاقاً: التراجع من البيان وحده
        let undone = OrganizerEngine::undo_from_journal(&root, &AtomicUsize::new(0)).unwrap();

        assert_eq!(undone.done.len(), plan.moves.len());
        assert!(undone.failed.is_empty());
        assert_eq!(tree(&root), before);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn undo_after_an_interrupted_run_restores_only_moved_files() {
        let root = temp_tree("interrupted", &["a/1.pdf", "a/2.pdf", "a/3.pdf"]);
        let plan = OrganizerEngine::plan(&root, &rebuild()).unwrap();
        // البرنامج توقف بعد نقل أول ملف فقط، والسجل يحتوي الخطة كاملة
        let partial = Plan {
            source: plan.source.clone(),
            moves: plan.moves[..1].to_vec(),
            new_dirs: plan.new_dirs.clone(),
            in_place: 0,
            kept_in_folders: 0,
            duplicates: 0,
            skipped: Vec::new(),
        };
        OrganizerEngine::execute(&partial, &AtomicUsize::new(0));

        let undone = undo(&plan, &plan.moves);

        assert_eq!((undone.done.len(), undone.unchanged, undone.failed.len()), (1, 2, 0));
        assert_eq!(tree(&root), ["a/1.pdf", "a/2.pdf", "a/3.pdf"]);
        assert!(!root.join("Documents").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn undo_never_overwrites_a_new_file() {
        let root = temp_tree("undo_clash", &["old/report.pdf"]);
        let (plan, result) = organize(&root, &rebuild());
        // المستخدم أنشأ ملفاً جديداً بنفس الاسم الأصلي بعد التنظيم
        fs::create_dir_all(root.join("old")).unwrap();
        fs::write(root.join("old/report.pdf"), b"new").unwrap();

        let undone = undo(&plan, &result.done);

        assert_eq!(undone.failed.len(), 1);
        assert_eq!(fs::read(root.join("old/report.pdf")).unwrap(), b"new");
        assert!(root.join("مستندات/PDF/report.pdf").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn execute_never_overwrites_a_file_created_after_planning() {
        let root = temp_tree("late_clash", &["report.pdf"]);
        let plan = OrganizerEngine::plan(&root, &rebuild()).unwrap();
        // ملف جديد ظهر في الوجهة بعد التخطيط
        let dest = plan.moves[0].to.clone();
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::write(&dest, b"other").unwrap();

        let result = OrganizerEngine::execute(&plan, &AtomicUsize::new(0));

        assert_eq!(result.failed.len(), 1);
        assert!(root.join("report.pdf").exists());
        assert_eq!(fs::read(&dest).unwrap(), b"other");
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn locked_file_is_reported_and_left_in_place() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = temp_tree("locked", &["locked.pdf", "free.pdf"]);
        // فتح الملف بدون مشاركة يمنع نقله وحذفه، كأنه مفتوح في برنامج آخر
        let lock = fs::OpenOptions::new().read(true).share_mode(0).open(root.join("locked.pdf")).unwrap();

        let (_, result) = organize(&root, &rebuild());

        assert_eq!((result.done.len(), result.failed.len()), (1, 1));
        assert!(root.join("locked.pdf").exists());
        assert!(!root.join("مستندات/PDF/locked.pdf").exists());
        drop(lock);
        fs::remove_dir_all(&root).unwrap();
    }
}
