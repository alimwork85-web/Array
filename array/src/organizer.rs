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

use crate::normalizer::{normalize_arabic, sanitize_filename};
use crate::safety::SafetyChecker;

// تُجمَّع التعبيرات مرة واحدة بدلاً من كل ملف
static RE_BRACKETS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[.*?\]|\(.*?\)|_|\bAUD-\w+\b").unwrap());
static RE_PIPES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\|+").unwrap());
static RE_SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
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
}

impl Default for Options {
    fn default() -> Self {
        Self {
            rename_files: true,
            speaker_keywords: DEFAULT_SPEAKER_KEYWORDS.iter().map(|k| k.to_string()).collect(),
        }
    }
}

/// نقل ملف واحد: من مكانه الأصلي إلى مكانه داخل Array_Organized
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlannedMove {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// خطة التنظيم الكاملة قبل نقل أي ملف (تُعرض للمعاينة ثم تُنفَّذ)
pub struct Plan {
    pub source: PathBuf,
    pub output_base: PathBuf,
    pub moves: Vec<PlannedMove>,
}

/// نتيجة التنفيذ أو التراجع
#[derive(Debug, Default)]
pub struct RunResult {
    pub done: Vec<PlannedMove>,
    pub failed: Vec<PlannedMove>,
    /// في التراجع: ملفات كانت في مكانها الأصلي أصلاً (تنظيم توقف قبل نقلها)
    pub unchanged: usize,
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
    raw_artist_or_group: String,
    /// اسم الملف الجديد بدون الامتداد (منظَّف أو الأصلي حسب الإعدادات)
    new_stem: String,
    /// العنوان المنظَّف، ويُستخدم لكشف السلاسل وأصحاب المقاطع
    clean_title: String,
    extension: String,
}

pub struct OrganizerEngine;

impl OrganizerEngine {
    pub fn get_file_type(ext: &str) -> FileType {
        match ext.to_lowercase().as_str() {
            "mp3" | "flac" | "m4a" | "wav" | "aac" | "ogg" | "wma" | "opus" => FileType::Audio,
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" => FileType::Image,
            "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" => FileType::Video,
            "pdf" | "docx" | "doc" | "xlsx" | "pptx" | "txt" | "epub" => FileType::Document,
            "zip" | "rar" | "7z" | "tar" | "gz" => FileType::Archive,
            _ => FileType::Other,
        }
    }

    fn clean_string(input: &str) -> String {
        // حذف الأقواس، الأكواد العشوائية [AUD-...], والأرقام التعريفية بين الأقواس
        let mut cleaned = RE_BRACKETS.replace_all(input, " ").to_string();
        cleaned = RE_PIPES.replace_all(&cleaned, " ").to_string();
        cleaned = RE_SPACES.replace_all(&cleaned, " ").to_string();

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

    /// حذف المجلدات التي أفرغها البرنامج فقط، صعوداً حتى المجلد المختار.
    /// المجلدات التي كانت فارغة قبل التشغيل لا تُلمس، و remove_dir يرفض أي مجلد غير فارغ.
    fn remove_emptied_dirs<'a>(files: impl Iterator<Item = &'a PathBuf>, source_path: &Path) {
        for file in files {
            let mut dir = file.parent();
            while let Some(d) = dir {
                if d == source_path || !d.starts_with(source_path) || fs::remove_dir(d).is_err() {
                    break;
                }
                dir = d.parent();
            }
        }
    }

    fn check_source(source_path: &Path) -> Result<PathBuf, String> {
        if !source_path.is_dir() || !SafetyChecker::is_safe_root(source_path) {
            return Err("The selected folder does not exist or is a protected system location.".to_string());
        }
        Ok(source_path.join("Array_Organized"))
    }

    /// كل الملفات المعروفة الأنواع التي سيتم نقلها
    fn collect_files(source_path: &Path, output_base: &Path) -> Vec<PathBuf> {
        // تخطّي مجلد المخرجات والمجلدات المخفية/مجلدات البرامج بكل ما فيها
        let walker = WalkDir::new(source_path).into_iter().filter_entry(|e| {
            e.depth() == 0
                || !e.file_type().is_dir()
                || (e.path() != output_base && !SafetyChecker::is_skipped_dir_name(&e.file_name().to_string_lossy()))
        });

        walker
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| !matches!(Self::get_file_type(ext), FileType::Other))
            })
            .map(|e| e.into_path())
            .collect()
    }

    /// حساب وجهة كل ملف بدون نقل أي شيء
    pub fn plan(source_path: &Path, options: &Options) -> Result<Plan, String> {
        let output_base = Self::check_source(source_path)?;
        let keywords: Vec<String> = options.speaker_keywords.iter().map(|k| Self::group_key(k.trim())).collect();

        let mut raw_files: Vec<RawFileInfo> = Vec::new();

        for path in Self::collect_files(source_path, &output_base) {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_string();
            let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();

            let (category, artist, clean_title) = match Self::get_file_type(&ext) {
                FileType::Audio => {
                    let (artist, title) = Self::extract_audio_info(&path);
                    ("Music", artist, title)
                }
                other => {
                    let category = match other {
                        FileType::Image => "Pictures",
                        FileType::Video => "Videos",
                        FileType::Document => "Documents",
                        _ => "Archives",
                    };
                    (category, "General".to_string(), Self::clean_string(&stem))
                }
            };
            // بدون إعادة تسمية يبقى الاسم والامتداد كما هما تماماً
            let (new_stem, extension) = if options.rename_files {
                (clean_title.clone(), ext.to_lowercase())
            } else {
                (stem, ext)
            };

            raw_files.push(RawFileInfo {
                path,
                category: category.to_string(),
                raw_artist_or_group: artist,
                new_stem,
                clean_title,
                extension,
            });
        }

        let moves = Self::plan_moves(raw_files, &output_base, &keywords);
        Ok(Plan { source: source_path.to_path_buf(), output_base, moves })
    }

    /// تنفيذ الخطة: نقل الملفات ثم حذف المجلدات التي أُفرغت. progress يزيد مع كل ملف.
    pub fn execute(plan: &Plan, progress: &AtomicUsize) -> RunResult {
        let mut result = RunResult::default();
        for m in &plan.moves {
            let ok = m.to.parent().is_some_and(|dir| fs::create_dir_all(dir).is_ok()) && Self::move_file(&m.from, &m.to).is_ok();
            if ok { result.done.push(m.clone()) } else { result.failed.push(m.clone()) }
            progress.fetch_add(1, Ordering::Relaxed);
        }
        Self::remove_emptied_dirs(result.done.iter().map(|m| &m.from), &plan.source);
        result
    }

    /// إرجاع الملفات إلى أماكنها الأصلية. لا يكتب فوق أي ملف موجود.
    pub fn undo(moves: &[PlannedMove], source_path: &Path, progress: &AtomicUsize) -> RunResult {
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
        Self::remove_emptied_dirs(result.done.iter().map(|m| &m.to), source_path);
        result
    }

    fn extract_audio_info(file_path: &Path) -> (String, String) {
        let raw_stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("Track");
        let mut artist = "".to_string();
        let mut title = raw_stem.to_string();

        if let Ok(tagged_file) = Probe::open(file_path).and_then(|p| p.read()) {
            if let Some(tag) = tagged_file.primary_tag().or_else(|| tagged_file.first_tag()) {
                if let Some(a) = tag.artist() {
                    let a_trim = a.trim();
                    if !a_trim.is_empty() && !a_trim.to_lowercase().contains("unknown") && !a_trim.starts_with("AUD-") {
                        artist = a_trim.to_string();
                    }
                }
                if let Some(t) = tag.title() {
                    if !t.trim().is_empty() {
                        title = t.trim().to_string();
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

        (artist, if clean_tit.is_empty() { "Audio Track".to_string() } else { clean_tit })
    }

    fn plan_moves(files: Vec<RawFileInfo>, output_base: &Path, keywords: &[String]) -> Vec<PlannedMove> {
        struct ProcessedItem {
            file: RawFileInfo,
            group_display_name: String,
            group_norm_key: String,
        }

        let mut processed_items: Vec<ProcessedItem> = Vec::new();
        let mut group_counts: HashMap<String, usize> = HashMap::new();
        let mut group_canonical_names: HashMap<String, String> = HashMap::new();
        // أسماء محجوزة داخل الخطة نفسها (بأحرف صغيرة لأن أسماء ويندوز لا تفرّق بين الحالة)
        let mut reserved: HashSet<String> = HashSet::new();
        let mut moves = Vec::new();

        for f in files {
            let (display_name, norm_key) = if f.category == "Music" {
                Self::extract_group_key(&f.raw_artist_or_group, &f.clean_title, keywords)
            } else {
                ("General".to_string(), "general".to_string())
            };

            if f.category == "Music" && norm_key != "single tracks" {
                *group_counts.entry(norm_key.clone()).or_insert(0) += 1;
                group_canonical_names.entry(norm_key.clone()).or_insert(display_name.clone());
            }

            processed_items.push(ProcessedItem {
                file: f,
                group_display_name: display_name,
                group_norm_key: norm_key,
            });
        }

        for item in processed_items {
            let f = item.file;
            let target_dir = if f.category == "Music" {
                let count = group_counts.get(&item.group_norm_key).cloned().unwrap_or(0);
                // شرط صارم: إنشاء المجلد فقط إذا كانت الملفات للمجموعة 3 أو أكثر
                if count >= 3 && item.group_norm_key != "single tracks" {
                    let canonical_folder_name = group_canonical_names
                        .get(&item.group_norm_key)
                        .unwrap_or(&item.group_display_name);
                    output_base.join("Music").join(canonical_folder_name)
                } else {
                    output_base.join("Music").join("Single Tracks")
                }
            } else {
                output_base.join(&f.category)
            };

            let mut dest_path = target_dir.join(format!("{}.{}", f.new_stem, f.extension));
            let mut counter = 1;

            // تجنب تكرار أسماء الملفات المتشابهة (الموجودة فعلاً أو المحجوزة في الخطة)
            while dest_path.exists() || reserved.contains(&dest_path.to_string_lossy().to_lowercase()) {
                dest_path = target_dir.join(format!("{} ({}).{}", f.new_stem, counter, f.extension));
                counter += 1;
            }
            reserved.insert(dest_path.to_string_lossy().to_lowercase());

            moves.push(PlannedMove { from: f.path, to: dest_path });
        }

        moves
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn keywords() -> Vec<String> {
        DEFAULT_SPEAKER_KEYWORDS.iter().map(|k| OrganizerEngine::group_key(k)).collect()
    }

    fn group(artist: &str, title: &str) -> (String, String) {
        OrganizerEngine::extract_group_key(artist, title, &keywords())
    }

    fn group_of(file_name: &str) -> String {
        let (artist, title) = OrganizerEngine::extract_audio_info(Path::new(file_name));
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

    #[test]
    fn plan_moves_nothing() {
        let root = temp_tree("plan", &["a/IMG_1.jpg", "IMG 1.jpg"]);
        let plan = OrganizerEngine::plan(&root, &Options::default()).unwrap();

        assert_eq!(plan.moves.len(), 2);
        assert!(root.join("a/IMG_1.jpg").exists() && !plan.output_base.exists());
        // اسمان يصبحان متطابقين بعد التنظيف يأخذان اسمين مختلفين داخل الخطة
        assert_ne!(plan.moves[0].to, plan.moves[1].to);

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
            "docs/report (final).pdf",
            ".git/notes.txt",
            "AppData/config.txt",
        ]);
        fs::create_dir_all(root.join(".git/empty")).unwrap();
        fs::create_dir_all(root.join("keep_me/inner")).unwrap();

        let (_, result) = organize(&root, &Options::default());
        let out = root.join("Array_Organized");

        assert_eq!(result.done.len(), 6);
        assert!(result.failed.is_empty());
        assert!(out.join("Music/الشيخ محمد/الشيخ محمد.mp3").exists());
        assert!(out.join("Music/الشيخ محمد/الشيخ محمد (1).mp3").exists());
        assert!(out.join("Music/الشيخ محمد/الشيخ محمد (2).mp3").exists());
        assert!(out.join("Music/Single Tracks/song one.mp3").exists());
        assert!(out.join("Pictures/IMG 001.jpg").exists());
        assert!(out.join("Documents/report.pdf").exists());
        // المجلدات المخفية ومجلدات البرامج لا تُلمس
        assert!(root.join(".git/notes.txt").exists());
        assert!(root.join(".git/empty").is_dir());
        assert!(root.join("AppData/config.txt").exists());
        // المجلدات التي أُفرغت تُحذف
        assert!(!root.join("a").exists() && !root.join("docs").exists());
        // المجلدات التي كانت فارغة قبل التشغيل تبقى
        assert!(root.join("keep_me/inner").is_dir());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn keeps_original_names_when_renaming_is_off() {
        let root = temp_tree("norename", &["docs/report (final).PDF", "IMG_001.jpg"]);
        let options = Options { rename_files: false, ..Options::default() };

        organize(&root, &options);

        let out = root.join("Array_Organized");
        assert!(out.join("Documents/report (final).PDF").exists());
        assert!(out.join("Pictures/IMG_001.jpg").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn undo_restores_files_and_folders() {
        let files = ["a/الحلقة 1 - الشيخ محمد.mp3", "a/b/IMG_1.jpg", "report.pdf"];
        let root = temp_tree("undo", &files);
        fs::create_dir_all(root.join("keep_me")).unwrap();

        let (plan, result) = organize(&root, &Options::default());
        assert!(!root.join("a").exists());

        let progress = AtomicUsize::new(0);
        let undone = OrganizerEngine::undo(&result.done, &plan.source, &progress);

        assert_eq!(undone.done.len(), 3);
        assert!(undone.failed.is_empty());
        for f in files {
            assert!(root.join(f).exists(), "{f} was not restored");
        }
        // مجلد المخرجات أُزيل لأنه فرغ، والمجلد الفارغ الأصلي باقٍ
        assert!(!plan.output_base.exists());
        assert!(root.join("keep_me").is_dir());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn undo_after_an_interrupted_run_restores_only_moved_files() {
        let root = temp_tree("interrupted", &["a/1.pdf", "a/2.pdf", "a/3.pdf"]);
        let plan = OrganizerEngine::plan(&root, &Options::default()).unwrap();
        // البرنامج توقف بعد نقل أول ملف فقط، والسجل يحتوي الخطة كاملة
        let partial = Plan { source: plan.source.clone(), output_base: plan.output_base.clone(), moves: plan.moves[..1].to_vec() };
        OrganizerEngine::execute(&partial, &AtomicUsize::new(0));

        let undone = OrganizerEngine::undo(&plan.moves, &plan.source, &AtomicUsize::new(0));

        assert_eq!((undone.done.len(), undone.unchanged, undone.failed.len()), (1, 2, 0));
        for f in ["a/1.pdf", "a/2.pdf", "a/3.pdf"] {
            assert!(root.join(f).exists(), "{f}");
        }
        assert!(!plan.output_base.exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn undo_never_overwrites_a_new_file() {
        let root = temp_tree("undo_clash", &["report.pdf"]);
        let (plan, result) = organize(&root, &Options::default());
        // المستخدم أنشأ ملفاً جديداً بنفس الاسم الأصلي بعد التنظيم
        fs::write(root.join("report.pdf"), b"new").unwrap();

        let undone = OrganizerEngine::undo(&result.done, &plan.source, &AtomicUsize::new(0));

        assert_eq!(undone.failed.len(), 1);
        assert_eq!(fs::read(root.join("report.pdf")).unwrap(), b"new");
        assert!(plan.output_base.join("Documents/report.pdf").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn execute_never_overwrites_a_file_created_after_planning() {
        let root = temp_tree("late_clash", &["report.pdf"]);
        let plan = OrganizerEngine::plan(&root, &Options::default()).unwrap();
        fs::create_dir_all(plan.output_base.join("Documents")).unwrap();
        fs::write(plan.output_base.join("Documents/report.pdf"), b"other").unwrap();

        let result = OrganizerEngine::execute(&plan, &AtomicUsize::new(0));

        assert_eq!(result.failed.len(), 1);
        assert!(root.join("report.pdf").exists());
        assert_eq!(fs::read(plan.output_base.join("Documents/report.pdf")).unwrap(), b"other");
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn locked_file_is_reported_and_left_in_place() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = temp_tree("locked", &["locked.pdf", "free.pdf"]);
        // فتح الملف بدون مشاركة يمنع نقله وحذفه، كأنه مفتوح في برنامج آخر
        let lock = fs::OpenOptions::new().read(true).share_mode(0).open(root.join("locked.pdf")).unwrap();

        let (_, result) = organize(&root, &Options::default());

        assert_eq!((result.done.len(), result.failed.len()), (1, 1));
        assert!(root.join("locked.pdf").exists());
        assert!(!root.join("Array_Organized/Documents/locked.pdf").exists());
        drop(lock);
        fs::remove_dir_all(&root).unwrap();
    }
}
