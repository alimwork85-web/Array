use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};
use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use lofty::tag::Accessor;
use regex::Regex;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::faces::{self, FaceCache};
use crate::grouping::{self, Assignment, Track, DEFAULT_AI_MARKERS};
use crate::naming;
use crate::normalizer::{fix_mojibake, looks_garbled, normalize_arabic, sanitize_filename, to_western_digits};
use crate::safety::SafetyChecker;

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
    /// ترتيب الصور المتفرقة في مجلدات الأشخاص الموجودة بالتعرّف على الوجوه (مع keep_existing_folders فقط)
    pub sort_by_faces: bool,
    /// مظهر الواجهة: "system" يتبع ويندوز، أو "light" أو "dark"
    pub appearance: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            rename_files: true,
            speaker_keywords: DEFAULT_SPEAKER_KEYWORDS.iter().map(|k| k.to_string()).collect(),
            ai_markers: DEFAULT_AI_MARKERS.iter().map(|k| k.to_string()).collect(),
            keep_existing_folders: true,
            sort_by_faces: true,
            appearance: "system".to_string(),
        }
    }
}

/// نقل ملف واحد: من مكانه الأصلي إلى مجلد فئته داخل المجلد المختار
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlannedMove {
    pub from: PathBuf,
    pub to: PathBuf,
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
    /// صور متفرقة نُسبت إلى مجلد شخص بالتعرّف على الوجه: المسار الأصلي → (الشخص، درجة التشابه)
    pub face_matches: HashMap<PathBuf, (String, f32)>,
    /// أسماء مجلدات الأشخاص التي تعلّم البرنامج وجوههم
    pub people: Vec<String>,
    /// ملاحظة عن التعرّف على الوجوه (مثل صور HEIC لم تُقرأ)
    pub face_note: Option<String>,
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

/// تقدّم مرحلة التخطيط (التعرّف على الوجوه قد يستغرق دقائق)
#[derive(Default)]
pub struct PlanProgress {
    pub stage: Mutex<String>,
    pub done: AtomicUsize,
    pub total: AtomicUsize,
}

impl PlanProgress {
    fn start(&self, stage: &str, total: usize) {
        *self.stage.lock().unwrap() = stage.to_string();
        self.done.store(0, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
    }
}

/// صور يمكن قراءتها للتعرّف على الوجوه (HEIC غير مدعومة حالياً)
fn face_readable(path: &Path) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .is_some_and(|e| matches!(e.as_str(), "jpg" | "jpeg" | "png" | "bmp" | "gif" | "webp" | "tif" | "tiff"))
}

/// صور مأخوذة بالتساوي من القائمة
fn spread(paths: &[PathBuf], n: usize) -> Vec<PathBuf> {
    let step = (paths.len() / n.max(1)).max(1);
    paths.iter().step_by(step).take(n).cloned().collect()
}

struct FaceSorting {
    matches: HashMap<PathBuf, (PathBuf, String, f32)>,
    people: Vec<String>,
    note: Option<String>,
}

pub enum FileType {
    Audio,
    Image,
    Video,
    Document,
    Archive,
    Other,
}

struct RawFileInfo {
    path: PathBuf,
    category: String,
    /// رقم الملف في قائمة الأدلة الصوتية (للموسيقى فقط)
    track_index: Option<usize>,
    /// اسم الملف الجديد بدون الامتداد (منظَّف أو الأصلي حسب الإعدادات)
    new_stem: String,
    /// الاسم لا يعبّر عن الملف (كود أو نص عشوائي): يُولَّد له اسم من السياق والتاريخ والمدة
    needs_name: bool,
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
            return Err("The selected folder does not exist.".to_string());
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
            _ => &["archives", "compressed", "أرشيف", "مضغوطات", "ملفات مضغوطة"],
        }
    }

    /// مجلد كل فئة داخل المجلد المختار: مجلد موجود يناسبها إن وُجد، وإلا مجلد جديد بالاسم الإنجليزي
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
        for category in ["Music", "Pictures", "Videos", "Documents", "Archives"] {
            let synonyms: Vec<String> = Self::category_synonyms(category).iter().map(|s| Self::group_key(s)).collect();
            if synonyms.contains(&root_key) {
                dirs.insert(category, source_path.to_path_buf());
                continue;
            }
            // الاسم الإنجليزي أولاً ثم بقية المرادفات بترتيبها
            let found = synonyms.iter().find_map(|syn| existing.iter().find(|(key, _)| key == syn).map(|(_, p)| p.clone()));
            let dir = found.unwrap_or_else(|| {
                let default = source_path.join(category);
                // اسم الفئة محجوز لملف أو لمجلد برنامج: نستخدم اسماً آخر بدل الكتابة داخله
                if default.exists() { source_path.join(format!("{} (Array)", category)) } else { default }
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
                e.path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| !matches!(Self::get_file_type(ext), FileType::Other))
            })
            .map(|e| e.into_path())
            .collect();
        (files, skipped)
    }

    /// حساب وجهة كل ملف بدون نقل أي شيء. كل الملفات في المجلدات الفرعية تُجمع ثم تُوزَّع
    /// على مجلدات الفئات داخل المجلد المختار نفسه (الموجودة أو الجديدة).
    #[cfg(test)]
    pub fn plan(source_path: &Path, options: &Options) -> Result<Plan, String> {
        Self::plan_with(source_path, options, &PlanProgress::default(), None)
    }

    /// مثل plan مع تقدّم للواجهة وملف ذاكرة بصمات الوجوه (حتى لا يُعاد تحليل الصور كل مرة)
    pub fn plan_with(source_path: &Path, options: &Options, progress: &PlanProgress, face_cache: Option<&Path>) -> Result<Plan, String> {
        Self::check_source(source_path)?;
        let keywords: Vec<String> = options.speaker_keywords.iter().map(|k| Self::group_key(k.trim())).collect();

        let mut raw_files: Vec<RawFileInfo> = Vec::new();
        let (files, skipped) = Self::collect_files(source_path);
        let category_dirs = Self::category_dirs(source_path);
        let music_dir = &category_dirs["Music"];
        let mut tracks: Vec<Track> = Vec::new();

        for path in files {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_string();
            let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();

            let (category, clean_name, track) = match Self::get_file_type(&ext) {
                FileType::Audio => {
                    let (artist, title, tag_title) = Self::extract_audio_info(&path);
                    let (speaker, speaker_key) = Self::extract_group_key(&artist, &title, &keywords);
                    let speaker = (speaker_key != "single tracks").then_some(speaker);
                    let track = grouping::read_track(&path, source_path, music_dir, &artist, &title, speaker.as_deref(), &options.ai_markers);
                    ("Music", Self::best_name(&stem, tag_title), Some(track))
                }
                other => {
                    let category = match other {
                        FileType::Image => "Pictures",
                        FileType::Video => "Videos",
                        FileType::Document => "Documents",
                        _ => "Archives",
                    };
                    (category, Self::clean_string(&stem), None)
                }
            };
            // بدون إعادة تسمية يبقى الاسم والامتداد كما هما تماماً
            let (new_stem, extension) = if options.rename_files {
                (clean_name, ext.to_lowercase())
            } else {
                (stem, ext)
            };

            let needs_name = options.rename_files && naming::is_junk_name(&new_stem);
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
                needs_name,
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
        let target_dir = |f: &RawFileInfo| match f.track_index.map(|i| &assignments[i]) {
            Some(Assignment::Folder(name)) => music_dir.join(name),
            Some(Assignment::Ai) => ai_dir.clone(),
            Some(Assignment::Misc) => misc_dir.clone(),
            None => category_dirs[f.category.as_str()].clone(),
        };
        // سياق الملف من مجلده الأصلي ("house lo [AUD" → "house lo") إن لم يكن مجلداً عاماً
        let category_paths: Vec<&PathBuf> = category_dirs.values().collect();
        let folder_context = |path: &Path| -> Option<String> {
            let parent = path.parent()?;
            if parent == source_path || category_paths.contains(&&parent.to_path_buf()) || parent == misc_dir {
                return None;
            }
            let name = parent.file_name()?.to_string_lossy().to_string();
            if matches!(name.trim().to_lowercase().as_str(), "single tracks" | "unknown") {
                return None;
            }
            let clean = Self::clean_string(&name);
            (!naming::is_junk_name(&clean)).then_some(clean)
        };
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

        let face_sorting = if options.keep_existing_folders && options.sort_by_faces {
            Self::sort_by_faces(&raw_files, &category_dirs["Pictures"], progress, face_cache)
        } else {
            FaceSorting { matches: HashMap::new(), people: Vec::new(), note: None }
        };

        let targeted: Vec<(RawFileInfo, PathBuf)> = raw_files
            .into_iter()
            .map(|mut f| {
                let face = face_sorting.matches.get(&f.path);
                let dir = face.map_or_else(|| target_dir(&f), |(dir, _, _)| dir.clone());
                if f.needs_name {
                    let group_context = match f.track_index.map(|i| &assignments[i]) {
                        Some(Assignment::Folder(name)) => Some(name.clone()),
                        Some(Assignment::Ai) => Some("AI Music".to_string()),
                        _ => face.map(|(_, person, _)| person.clone()),
                    };
                    let category_word = match f.category.as_str() {
                        "Music" => "Audio",
                        "Pictures" => "Photo",
                        "Videos" => "Video",
                        "Documents" => "Document",
                        _ => "Archive",
                    };
                    let context = group_context
                        .or_else(|| folder_context(&f.path))
                        .or_else(|| f.placeholder_context.clone())
                        .unwrap_or_else(|| category_word.to_string());
                    f.new_stem = sanitize_filename(&naming::generated_name(&context, &f.path));
                }
                (f, dir)
            })
            .collect();

        let (moves, in_place) = Self::plan_moves(targeted);

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

        // الملاحظات تخص فقط الصور التي ستُنقل فعلاً (لا التي هي في مكانها)
        let moved_from: HashSet<&PathBuf> = moves.iter().map(|m| &m.from).collect();
        let face_matches = face_sorting
            .matches
            .into_iter()
            .filter(|(from, _)| moved_from.contains(from))
            .map(|(from, (_, person, score))| (from, (person, score)))
            .collect();

        Ok(Plan {
            source: source_path.to_path_buf(),
            moves,
            new_dirs,
            in_place,
            kept_in_folders,
            face_matches,
            people: face_sorting.people,
            face_note: face_sorting.note,
            skipped,
        })
    }

    /// يتعلّم وجوه أصحاب المجلدات الموجودة (Ali، Mom...) ثم يقترح لكل صورة متفرقة مجلد صاحبها.
    /// لا يُقترح شيء إلا لصورة فيها شخص معروف واحد بتشابه واضح؛ غير ذلك تبقى الصورة مكانها.
    fn sort_by_faces(files: &[RawFileInfo], pictures_dir: &Path, progress: &PlanProgress, cache_path: Option<&Path>) -> FaceSorting {
        let mut sorting = FaceSorting { matches: HashMap::new(), people: Vec::new(), note: None };
        let loose: Vec<PathBuf> = files.iter().filter(|f| f.category == "Pictures").map(|f| f.path.clone()).collect();
        let readable: Vec<PathBuf> = loose.iter().filter(|p| face_readable(p)).cloned().collect();
        if readable.is_empty() {
            return sorting;
        }

        // مجلدات الأشخاص المرشحة: مجلدات فرعية بأسماء ليست سنوات ولا أسماء عامة
        let mut folders: Vec<(String, PathBuf, Vec<PathBuf>)> = fs::read_dir(pictures_dir)
            .map(|entries| entries.flatten().collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.metadata().is_ok_and(|m| m.is_dir() && !SafetyChecker::is_hidden_or_system(&m)))
            .map(|e| (e.file_name().to_string_lossy().to_string(), e.path()))
            .filter(|(name, dir)| !faces::is_generic_folder_name(name) && !SafetyChecker::is_skipped_dir_name(name) && SafetyChecker::program_folder(dir).is_none())
            .filter_map(|(name, dir)| {
                let mut images: Vec<PathBuf> = fs::read_dir(&dir).ok()?.flatten().map(|e| e.path()).filter(|p| face_readable(p)).collect();
                images.sort();
                (images.len() >= 10).then(|| (name, dir, spread(&images, 150)))
            })
            .collect();
        folders.sort_by(|a, b| a.0.cmp(&b.0));
        if folders.is_empty() {
            return sorting;
        }

        let cache = Mutex::new(cache_path.map(FaceCache::load).unwrap_or_default());
        let learn_total: usize = folders.iter().map(|f| f.2.len()).sum();
        progress.start("Learning faces from your folders", learn_total);
        let mut people = Vec::new();
        for (name, dir, sample) in &folders {
            match faces::analyze_all(sample, &cache, &progress.done) {
                Ok(analyzed) => {
                    let photos: Vec<_> = sample.iter().map(|p| analyzed.get(p).cloned().flatten()).collect();
                    people.extend(faces::learn_person(name, dir, &photos));
                }
                Err(err) => {
                    sorting.note = Some(format!("Face sorting is off: {err}"));
                    return sorting;
                }
            }
        }
        faces::remove_conflicts(&mut people);
        sorting.people = people.iter().map(|p| p.name.clone()).collect();

        if !people.is_empty() {
            progress.start("Recognizing faces in loose photos", readable.len());
            if let Ok(analyzed) = faces::analyze_all(&readable, &cache, &progress.done) {
                for path in &readable {
                    if let Some(Some(found)) = analyzed.get(path) {
                        if let Some((who, score)) = faces::photo_owner(found, &people, faces::MATCH) {
                            sorting.matches.insert(path.clone(), (people[who].dir.clone(), people[who].name.clone(), score));
                        }
                    }
                }
            }
        }
        if let Some(path) = cache_path {
            cache.lock().unwrap().save(path);
        }
        let unreadable = loose.len() - readable.len();
        if unreadable > 0 {
            sorting.note = Some(format!("{} photos (HEIC/HEIF) could not be read for face sorting.", unreadable));
        }
        sorting
    }

    /// تنفيذ الخطة: نقل الملفات ثم حذف المجلدات التي أُفرغت. progress يزيد مع كل ملف.
    pub fn execute(plan: &Plan, progress: &AtomicUsize) -> RunResult {
        let mut result = RunResult::default();
        for m in &plan.moves {
            let ok = m.to.parent().is_some_and(|dir| fs::create_dir_all(dir).is_ok()) && Self::move_file(&m.from, &m.to).is_ok();
            if ok { result.done.push(m.clone()) } else { result.failed.push(m.clone()) }
            progress.fetch_add(1, Ordering::Relaxed);
        }
        Self::remove_emptied_dirs(result.done.iter().map(|m| &m.from), &plan.source, |_| true);
        result
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

    /// يرجع (عمليات النقل، عدد الملفات الموجودة أصلاً في مكانها الصحيح)
    fn plan_moves(files: Vec<(RawFileInfo, PathBuf)>) -> (Vec<PlannedMove>, usize) {
        // أسماء محجوزة داخل الخطة نفسها (بأحرف صغيرة لأن أسماء ويندوز لا تفرّق بين الحالة)
        let mut reserved: HashSet<String> = HashSet::new();
        let mut moves = Vec::new();
        let mut in_place = 0;
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
                moves.push(PlannedMove { from: f.path, to: dest_path });
            }
        }

        (moves, in_place)
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
    fn tree(root: &Path) -> Vec<String> {
        let mut files: Vec<String> = WalkDir::new(root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.path().strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
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
    fn empties_subfolders_and_organizes_inside_the_selected_folder() {
        // المثال: مجلد "صور" موجود يُعاد استخدامه، والفئات الأخرى تُنشأ، و"رحلة 2023" يُفرَّغ ويُحذف
        let root = temp_tree("inplace", &["رحلة 2023/sea.jpg", "رحلة 2023/clip.mp4", "صور/cat.png", "cv.pdf"]);

        let (plan, result) = organize(&root, &rebuild());

        assert!(result.failed.is_empty());
        assert_eq!(tree(&root), ["Documents/cv.pdf", "Videos/clip.mp4", "صور/cat.png", "صور/sea.jpg"]);
        assert!(!root.join("رحلة 2023").exists());
        assert_eq!(plan.new_dirs, [root.join("Documents"), root.join("Videos")]);
        // cat.png في مكانه الصحيح أصلاً فلا يتحرك
        assert_eq!(plan.in_place, 1);
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

        assert!(root.join("Photos/sea.jpg").exists());
        assert!(root.join("الموسيقى/منوعات/song.mp3").exists());
        assert!(root.join("مستندات/cv.docx").exists());
        assert!(!root.join("Pictures").exists() && !root.join("Music").exists() && !root.join("Documents").exists());
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

        assert!(root.join("Music (Array)/منوعات/song.mp3").exists());
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
        assert!(root.join("Music/الشيخ محمد/الحلقة 1 - الشيخ محمد.mp3").exists(), "{:?}", tree(&root));
        assert!(root.join("Music/الشيخ محمد/الحلقة 2 - الشيخ محمد.mp3").exists());
        assert!(root.join("Music/الشيخ محمد/الحلقة 3 - الشيخ محمد.mp3").exists());
        assert!(root.join("Music/منوعات/song one.mp3").exists());
        assert!(root.join("Pictures/IMG 001.jpg").exists());
        // الأقواس ذات المعنى تبقى في الاسم
        assert!(root.join("Documents/report (final).pdf").exists());
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

        // Photos مجلد صور موجود: sea.jpg في مكانه، و loose.jpg وحده ينتقل إليه
        let moved: Vec<_> = plan.moves.iter().map(|m| m.from.strip_prefix(&root).unwrap().to_path_buf()).collect();
        assert_eq!(moved, vec![PathBuf::from("loose.jpg")]);
        assert_eq!(plan.moves[0].to, root.join("Photos").join("loose.jpg"));
        let mut skipped: Vec<_> = plan.skipped.iter().map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        skipped.sort();
        assert_eq!(skipped, ["Game", "Lib", "MyProject", "MyTool", "SavedPage", "Scripts"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn refuses_to_organize_a_program_folder_itself() {
        let root = temp_tree("tool_root", &["tool.exe", "data.txt"]);
        let err = OrganizerEngine::plan(&root, &rebuild()).err().unwrap();
        assert!(err.contains("program"), "{err}");
        assert!(root.join("data.txt").exists());
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
        assert_eq!(targets, ["Archives", "Documents", "Music", "Pictures", "Videos"]);
        // اختصارات وقواعد بيانات وملفات غير معروفة لا تُلمس
        assert_eq!(plan.moves.len(), 5);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn meaningless_names_get_a_name_from_their_context() {
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
        // مجلد "house lo [AUD" يُنظَّف اسمه، وملفاته تأخذ اسماً من سياقها
        assert_eq!(files.iter().filter(|f| f.starts_with("house lo/house lo - 20")).count(), 2, "{files:?}");
        assert!(files.iter().any(|f| f.starts_with("منوعات/Audio - 20")), "{files:?}");
        assert!(files.iter().any(|f| f.starts_with("Videos/Video - 20")), "{files:?}");
        // الاسم المفهوم لا يتغير
        assert!(files.contains(&"منوعات/Ma Yehtaj.mp3".to_string()), "{files:?}");
        assert!(!files.iter().any(|f| f.contains("]")), "{files:?}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn meaningless_names_stay_when_renaming_is_off() {
        let root = temp_tree("junk_off", &["x/4749d951].mp3"]);
        organize(&root, &Options { rename_files: false, ..rebuild() });
        assert_eq!(tree(&root), ["Music/منوعات/4749d951].mp3"]);
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
        assert!(files.contains(&"IMG 0612.jpg".to_string()), "{files:?}");
        assert!(files.iter().any(|f| f.starts_with("Photo - 20")), "{files:?}");
        assert!(files.contains(&"Videos/trip.mp4".to_string()), "{files:?}");
        assert!(files.contains(&"Documents/cv.pdf".to_string()), "{files:?}");
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
        assert!(files.contains(&"Pictures/sea.jpg".to_string()), "{files:?}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn keeps_original_names_when_renaming_is_off() {
        let root = temp_tree("norename", &["work/report (final).PDF", "IMG_001.jpg"]);
        let options = Options { rename_files: false, ..rebuild() };

        organize(&root, &options);

        assert!(root.join("Documents/report (final).PDF").exists());
        assert!(root.join("Pictures/IMG_001.jpg").exists());
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
            face_matches: HashMap::new(),
            people: Vec::new(),
            face_note: None,
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
        assert!(root.join("Documents/report.pdf").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn execute_never_overwrites_a_file_created_after_planning() {
        let root = temp_tree("late_clash", &["report.pdf"]);
        let plan = OrganizerEngine::plan(&root, &rebuild()).unwrap();
        fs::create_dir_all(root.join("Documents")).unwrap();
        fs::write(root.join("Documents/report.pdf"), b"other").unwrap();

        let result = OrganizerEngine::execute(&plan, &AtomicUsize::new(0));

        assert_eq!(result.failed.len(), 1);
        assert!(root.join("report.pdf").exists());
        assert_eq!(fs::read(root.join("Documents/report.pdf")).unwrap(), b"other");
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
        assert!(!root.join("Documents/locked.pdf").exists());
        drop(lock);
        fs::remove_dir_all(&root).unwrap();
    }
}
