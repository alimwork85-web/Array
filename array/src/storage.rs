use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::organizer::{Options, PlannedMove};

/// سجل آخر عملية تنظيم، ويُكتب قبل نقل أي ملف حتى يمكن التراجع حتى لو توقف البرنامج فجأة
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct UndoLog {
    pub source: PathBuf,
    pub moves: Vec<PlannedMove>,
    /// مجلدات أنشأها التنظيم، فيحذفها التراجع إذا فرغت (المجلدات الأصلية لا تُحذف)
    #[serde(default)]
    pub created_dirs: Vec<PathBuf>,
}

/// ملفات البرنامج في %APPDATA%\Array: الإعدادات وسجل التراجع
#[derive(Clone)]
pub struct Storage {
    dir: PathBuf,
}

impl Storage {
    pub fn default_location() -> Self {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)))
            .unwrap_or_else(|| PathBuf::from("."));
        Self::at(base.join("Array"))
    }

    pub fn at(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn settings_path(&self) -> PathBuf {
        self.dir.join("settings.json")
    }

    /// ذاكرة بصمات الوجوه (حتى لا تُحلَّل نفس الصورة مرتين)
    pub fn face_cache_path(&self) -> PathBuf {
        self.dir.join("face_cache.json")
    }

    fn undo_path(&self) -> PathBuf {
        self.dir.join("last_organize.json")
    }

    /// الكتابة في ملف مؤقت ثم استبداله، حتى لا يبقى ملف نصف مكتوب إذا انقطع البرنامج
    fn write_atomic(&self, path: &Path, contents: &str) -> Result<(), String> {
        let tmp = path.with_extension("json.tmp");
        fs::create_dir_all(&self.dir)
            .and_then(|_| fs::write(&tmp, contents))
            .and_then(|_| fs::rename(&tmp, path))
            .map_err(|e| format!("Could not save {}: {}", path.display(), e))
    }

    /// إذا لم يوجد الملف يُنشأ بالقيم الافتراضية ليتمكن المستخدم من تعديله.
    /// إذا كان تالفاً يُرجع خطأ ولا يُكتب فوقه حتى لا تضيع تعديلات المستخدم.
    pub fn load_settings(&self) -> Result<Options, String> {
        let path = self.settings_path();
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| format!("settings.json is not valid ({}). Default settings are used until it is fixed.", e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let defaults = Options::default();
                self.save_settings(&defaults)?;
                Ok(defaults)
            }
            Err(e) => Err(format!("Could not read settings.json: {}", e)),
        }
    }

    pub fn save_settings(&self, options: &Options) -> Result<(), String> {
        let text = serde_json::to_string_pretty(options).map_err(|e| e.to_string())?;
        self.write_atomic(&self.settings_path(), &text)
    }

    pub fn save_undo(&self, log: &UndoLog) -> Result<(), String> {
        let text = serde_json::to_string(log).map_err(|e| e.to_string())?;
        self.write_atomic(&self.undo_path(), &text)
    }

    pub fn load_undo(&self) -> Result<Option<UndoLog>, String> {
        match fs::read_to_string(self.undo_path()) {
            Ok(text) => serde_json::from_str(&text).map(Some).map_err(|e| format!("The undo record is damaged: {}", e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("Could not read the undo record: {}", e)),
        }
    }

    pub fn clear_undo(&self) -> Result<(), String> {
        match fs::remove_file(self.undo_path()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("Could not clear the undo record: {}", e)),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::organizer::tests::temp_tree;

    #[test]
    fn missing_settings_file_is_created_with_defaults() {
        let dir = temp_tree("settings_new", &[]);
        let storage = Storage::at(dir.join("Array"));

        assert_eq!(storage.load_settings().unwrap(), Options::default());
        let text = fs::read_to_string(storage.settings_path()).unwrap();
        assert!(text.contains("الشيخ"), "Arabic keywords are written readably: {text}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn user_edits_are_loaded_and_partial_files_get_defaults() {
        let dir = temp_tree("settings_edit", &[]);
        let storage = Storage::at(dir.clone());
        fs::write(storage.settings_path(), r#"{ "speaker_keywords": ["القارئ"] }"#).unwrap();

        let options = storage.load_settings().unwrap();
        assert_eq!(options.speaker_keywords, vec!["القارئ".to_string()]);
        assert!(options.rename_files);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn broken_settings_file_is_reported_and_left_untouched() {
        let dir = temp_tree("settings_bad", &[]);
        let storage = Storage::at(dir.clone());
        fs::write(storage.settings_path(), "{ broken").unwrap();

        assert!(storage.load_settings().is_err());
        assert_eq!(fs::read_to_string(storage.settings_path()).unwrap(), "{ broken");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undo_log_round_trip_and_clear() {
        let dir = temp_tree("undo_log", &[]);
        let storage = Storage::at(dir.clone());
        assert_eq!(storage.load_undo().unwrap(), None);

        let log = UndoLog {
            source: dir.join("موسيقى"),
            moves: vec![PlannedMove { from: dir.join("a.mp3"), to: dir.join("Music/a.mp3") }],
            created_dirs: vec![dir.join("Music")],
        };
        storage.save_undo(&log).unwrap();
        assert_eq!(storage.load_undo().unwrap(), Some(log));

        storage.clear_undo().unwrap();
        assert_eq!(storage.load_undo().unwrap(), None);
        storage.clear_undo().unwrap(); // مسح سجل غير موجود ليس خطأ
        fs::remove_dir_all(&dir).unwrap();
    }
}
