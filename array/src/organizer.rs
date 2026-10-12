use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use lofty::tag::Accessor;
use regex::Regex;
use walkdir::WalkDir;

use crate::normalizer::{normalize_arabic, sanitize_filename};
use crate::safety::SafetyChecker;

// تُجمَّع التعبيرات مرة واحدة بدلاً من كل ملف
static RE_BRACKETS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[.*?\]|\(.*?\)|_|\bAUD-\w+\b").unwrap());
static RE_PIPES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\|+").unwrap());
static RE_SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static RE_EPISODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(الحلقة|حلقة|part|ep|episode)\s*\d+").unwrap());

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

    /// استخراج المجموعات والسلاسل الذكي
    fn extract_group_key(artist_raw: &str, title_raw: &str) -> (String, String) {
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
            if norm_suf.chars().count() > 3 && (norm_suf.contains("الشيخ") || norm_suf.contains("دكتور") || norm_suf.contains("فنان") || norm_suf.contains("ادهم")) {
                return (suffix.to_string(), norm_suf);
            }
        }

        ("Single Tracks".to_string(), "single tracks".to_string())
    }

    fn move_file(src: &Path, dst: &Path) -> std::io::Result<()> {
        if src == dst {
            return Ok(());
        }
        if let Err(_) = fs::rename(src, dst) {
            fs::copy(src, dst)?;
            fs::remove_file(src)?;
        }
        Ok(())
    }

    fn remove_empty_dirs(dir: &Path, skip_dir: &Path) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && path != skip_dir {
                    Self::remove_empty_dirs(&path, skip_dir);
                    let _ = fs::remove_dir(&path);
                }
            }
        }
    }

    pub fn process_directory(source_path: &Path) -> Result<String, String> {
        if !source_path.exists() || !SafetyChecker::is_safe_to_process(source_path) {
            return Err("المسار المحدد غير آمن أو غير موجود!".to_string());
        }

        let output_base = source_path.join("Array_Organized");
        if let Err(e) = fs::create_dir_all(&output_base) {
            return Err(format!("فشل إنشاء مجلد المخرجات: {}", e));
        }

        let mut raw_files: Vec<RawFileInfo> = Vec::new();

        for entry in WalkDir::new(source_path).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();

            if path.starts_with(&output_base) || !SafetyChecker::is_safe_to_process(path) {
                continue;
            }

            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    let file_type = Self::get_file_type(ext);
                    let ext_lower = ext.to_lowercase();
                    let stem = path.file_stem().unwrap_or_default().to_string_lossy();

                    match file_type {
                        FileType::Audio => {
                            let (artist, title) = Self::extract_audio_info(path);
                            raw_files.push(RawFileInfo {
                                path: path.to_path_buf(),
                                category: "Music".to_string(),
                                raw_artist_or_group: artist,
                                clean_title: title,
                                extension: ext_lower,
                            });
                        }
                        FileType::Other => {}
                        other => {
                            let category = match other {
                                FileType::Image => "Pictures",
                                FileType::Video => "Videos",
                                FileType::Document => "Documents",
                                _ => "Archives",
                            };
                            raw_files.push(RawFileInfo {
                                path: path.to_path_buf(),
                                category: category.to_string(),
                                raw_artist_or_group: "General".to_string(),
                                clean_title: Self::clean_string(&stem),
                                extension: ext_lower,
                            });
                        }
                    }
                }
            }
        }

        let count = raw_files.len();
        Self::organize_files(raw_files, &output_base)?;
        Self::remove_empty_dirs(source_path, &output_base);

        Ok(format!("Successfully moved and organized {} files into {:?}", count, output_base))
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

    fn organize_files(files: Vec<RawFileInfo>, output_base: &Path) -> Result<(), String> {
        struct ProcessedItem {
            file: RawFileInfo,
            group_display_name: String,
            group_norm_key: String,
        }

        let mut processed_items: Vec<ProcessedItem> = Vec::new();
        let mut group_counts: HashMap<String, usize> = HashMap::new();
        let mut group_canonical_names: HashMap<String, String> = HashMap::new();

        for f in files {
            let (display_name, norm_key) = if f.category == "Music" {
                Self::extract_group_key(&f.raw_artist_or_group, &f.clean_title)
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

            if let Err(e) = fs::create_dir_all(&target_dir) {
                return Err(format!("فشل إنشاء المجلد: {}", e));
            }

            let mut new_filename = format!("{}.{}", f.clean_title, f.extension);
            let mut dest_path = target_dir.join(&new_filename);
            let mut counter = 1;

            // تجنب تكرار أسماء الملفات المتشابهة
            while dest_path.exists() && dest_path != f.path {
                new_filename = format!("{} ({}).{}", f.clean_title, counter, f.extension);
                dest_path = target_dir.join(&new_filename);
                counter += 1;
            }

            let _ = Self::move_file(&f.path, &dest_path);
        }

        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn group_of(file_name: &str) -> String {
        let (artist, title) = OrganizerEngine::extract_audio_info(Path::new(file_name));
        OrganizerEngine::extract_group_key(&artist, &title).0
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
        let (name, _) = OrganizerEngine::extract_group_key("", "محاضرة - أدهم الشرقاوي");
        assert_eq!(name, "أدهم الشرقاوي");
    }

    #[test]
    fn artist_key_ignores_case_and_hamza() {
        let (_, a) = OrganizerEngine::extract_group_key("Amr Diab", "x");
        let (_, b) = OrganizerEngine::extract_group_key("amr diab", "x");
        assert_eq!(a, b);
        let (_, c) = OrganizerEngine::extract_group_key("أحمد", "x");
        let (_, d) = OrganizerEngine::extract_group_key("احمد", "x");
        assert_eq!(c, d);
    }

    #[test]
    fn short_arabic_artist_counted_in_letters() {
        // "علي" ثلاثة أحرف (6 بايت) فهو اسم صالح، و"عل" حرفان فقط
        assert_eq!(OrganizerEngine::extract_group_key("علي", "x").0, "علي");
        assert_eq!(OrganizerEngine::extract_group_key("عل", "x").0, "Single Tracks");
    }
}
