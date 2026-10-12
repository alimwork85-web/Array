use std::path::{Component, Path};

pub struct SafetyChecker;

/// مجلدات نظام لا يجوز اختيارها كمجلد للتنظيم (أول جزء بعد جذر القرص)
const PROTECTED_TOP_LEVEL: [&str; 4] = ["windows", "program files", "program files (x86)", "programdata"];

/// مجلدات تُتخطّى أثناء المسح لأن نقل ملفاتها يكسر برامج أو مشاريع
const SKIPPED_DIRS: [&str; 4] = ["appdata", "$recycle.bin", "system volume information", "node_modules"];

impl SafetyChecker {
    /// هل يجوز تنظيم هذا المجلد كاملاً؟ يمنع جذر أي قرص ومجلدات النظام
    pub fn is_safe_root(path: &Path) -> bool {
        let mut names = path.components().filter_map(|c| match c {
            Component::Normal(name) => Some(name.to_string_lossy().to_lowercase()),
            _ => None,
        });

        match names.next() {
            None => false, // جذر قرص مثل C:\ أو D:\
            Some(first) => !PROTECTED_TOP_LEVEL.contains(&first.as_str()) && !Self::is_skipped_dir_name(&first),
        }
    }

    /// هل يجب تخطّي هذا المجلد الفرعي (مخفي أو مجلد نظام/برامج)؟
    pub fn is_skipped_dir_name(name: &str) -> bool {
        let lower = name.to_lowercase();
        lower.starts_with('.') || SKIPPED_DIRS.contains(&lower.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_roots_and_system_folders_are_blocked() {
        assert!(!SafetyChecker::is_safe_root(Path::new("C:\\")));
        assert!(!SafetyChecker::is_safe_root(Path::new("D:\\")));
        assert!(!SafetyChecker::is_safe_root(Path::new("C:\\Windows\\System32")));
        assert!(!SafetyChecker::is_safe_root(Path::new("C:\\Program Files\\App")));
        assert!(!SafetyChecker::is_safe_root(Path::new("C:\\ProgramData")));
    }

    #[test]
    fn user_folders_are_allowed() {
        assert!(SafetyChecker::is_safe_root(Path::new("C:\\Users\\me\\Downloads")));
        assert!(SafetyChecker::is_safe_root(Path::new("D:\\Music")));
        // كلمة windows داخل اسم مجلد عادي لا تمنعه
        assert!(SafetyChecker::is_safe_root(Path::new("C:\\Users\\me\\Downloads\\windows-wallpapers")));
    }

    #[test]
    fn hidden_and_app_folders_are_skipped() {
        assert!(SafetyChecker::is_skipped_dir_name(".git"));
        assert!(SafetyChecker::is_skipped_dir_name("AppData"));
        assert!(SafetyChecker::is_skipped_dir_name("node_modules"));
        assert!(!SafetyChecker::is_skipped_dir_name("Downloads"));
    }
}
