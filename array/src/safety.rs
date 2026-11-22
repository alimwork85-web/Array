use std::fs;
use std::path::{Component, Path};

use crate::i18n::tr;

pub struct SafetyChecker;

/// مجلدات نظام لا يجوز اختيارها ولا تنظيم أي شيء داخلها (أول جزء بعد جذر القرص)
const PROTECTED_TOP_LEVEL: [&str; 14] = [
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "recovery",
    "perflogs",
    "boot",
    "efi",
    "intel",
    "amd",
    "nvidia",
    "msocache",
    "config.msi",
    "documents and settings",
];

/// مجلدات تُتخطّى أثناء المسح لأن نقل ملفاتها يكسر برامج أو مشاريع
const SKIPPED_DIRS: [&str; 7] = [
    "appdata",
    "$recycle.bin",
    "system volume information",
    "node_modules",
    "__pycache__",
    "site-packages",
    "venv",
];

/// ملفات أو مجلدات وجودها يعني أن المجلد مشروع برمجي
const PROJECT_MARKERS: [&str; 33] = [
    ".git", ".svn", ".hg", ".vscode", ".idea", ".vs", ".claude", ".venv",
    ".gitignore", ".gitattributes", ".editorconfig",
    "cargo.toml", "package.json", "pyproject.toml", "requirements.txt", "pipfile", "setup.cfg",
    "go.mod", "pom.xml", "build.gradle", "build.gradle.kts", "settings.gradle", "cmakelists.txt",
    "makefile", "composer.json", "gemfile", "dockerfile", "docker-compose.yml", "compose.yaml",
    "tsconfig.json", "pubspec.yaml", "pyvenv.cfg", "cachedir.tag",
];

/// امتدادات ملفات المشاريع وحلول بيئات التطوير
const PROJECT_EXTENSIONS: [&str; 10] = ["sln", "csproj", "vbproj", "fsproj", "vcxproj", "xcodeproj", "uproject", "pro", "iml", "ipynb"];

/// مكتبات وتعريفات يحتاجها برنامج لكي يعمل
const LIBRARY_EXTENSIONS: [&str; 7] = ["dll", "sys", "ocx", "cpl", "drv", "so", "dylib"];

/// برامج ومثبّتات
const PROGRAM_EXTENSIONS: [&str; 6] = ["exe", "msi", "msix", "appx", "appxbundle", "com"];

/// سكريبتات وكود مصدري وصفحات ويب: الملفات المجاورة لها غالباً جزء منها
const SCRIPT_EXTENSIONS: [&str; 38] = [
    "bat", "cmd", "ps1", "psm1", "psd1", "vbs", "vbe", "wsf", "ahk", "sh", "bash",
    "py", "pyw", "js", "mjs", "cjs", "ts", "jsx", "tsx", "vue", "rb", "pl", "php", "lua", "r", "jar",
    "rs", "go", "c", "cpp", "h", "hpp", "cs", "java", "kt", "swift", "html", "css",
];

/// ملفات إعدادات تعني أن المجلد يخص برنامجاً
const CONFIG_EXTENSIONS: [&str; 4] = ["ini", "cfg", "conf", "config"];

/// مجلدات المستخدم المعتادة: وجود ملف تثبيت فيها (مثل Downloads) لا يعني أنها مجلد برنامج
const USER_LIBRARY_FOLDERS: [&str; 6] = ["downloads", "desktop", "documents", "pictures", "music", "videos"];

/// لماذا يُعتبر المجلد خاصاً ببرنامج أو مشروع
#[derive(Debug, Clone, PartialEq)]
pub enum ProgramFolder {
    Project(String),
    Library(String),
    Script(String),
    Program(String),
    Config(String),
}

impl ProgramFolder {
    pub fn reason(&self) -> String {
        match self {
            Self::Project(name) => format!("{} ({})", tr("software project", "مشروع برمجي"), name),
            Self::Library(name) => format!("{} ({})", tr("program files", "ملفات برنامج"), name),
            Self::Script(name) => format!("{} ({})", tr("scripts or code", "سكريبتات أو كود"), name),
            Self::Program(name) => format!("{} ({})", tr("program", "برنامج"), name),
            Self::Config(name) => format!("{} ({})", tr("program settings", "إعدادات برنامج"), name),
        }
    }
}

const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

impl SafetyChecker {
    /// هل يجوز تنظيم هذا المجلد كاملاً؟ يمنع جذر أي قرص ومجلدات النظام ومجلد Users كله
    pub fn is_safe_root(path: &Path) -> bool {
        let names: Vec<String> = path
            .components()
            .filter_map(|c| match c {
                Component::Normal(name) => Some(name.to_string_lossy().to_lowercase()),
                _ => None,
            })
            .collect();

        match names.first() {
            None => false, // جذر قرص مثل C:\ أو D:\
            Some(first) if PROTECTED_TOP_LEVEL.contains(&first.as_str()) => false,
            Some(first) if first == "users" && names.len() == 1 => false, // كل حسابات المستخدمين
            // داخل AppData أو مجلد مخفي (.git, .vscode...) بيانات برامج؛ مجلد Temp مستثنى لأنه مؤقت
            Some(_) => path.starts_with(std::env::temp_dir()) || !names.iter().any(|n| Self::is_skipped_dir_name(n)),
        }
    }

    /// فحص المجلد المختار: يُرفض إذا كان هو نفسه برنامجاً أو مشروعاً
    pub fn check_root(path: &Path) -> Result<(), String> {
        if !Self::is_safe_root(path) {
            return Err(tr(
                "The selected folder does not exist or is a protected system location.",
                "المجلد المختار غير موجود أو هو مجلد نظام محمي.",
            )
            .to_string());
        }
        let is_user_library = path
            .file_name()
            .is_some_and(|n| USER_LIBRARY_FOLDERS.contains(&n.to_string_lossy().to_lowercase().as_str()));
        match Self::program_folder(path) {
            None => Ok(()),
            // ملف تثبيت داخل Downloads أو Desktop لا يجعلها مجلد برنامج
            Some(ProgramFolder::Program(_) | ProgramFolder::Config(_)) if is_user_library => Ok(()),
            // مجلد فيه مثبّتات فقط (setup.exe) ليس برنامجاً: المثبّتات فئة تُرتَّب
            Some(ProgramFolder::Program(_)) if Self::only_installers(path) => Ok(()),
            Some(kind) => Err(if crate::i18n::ar() {
                format!("هذا المجلد فيه {}. نقل ملفاته قد يوقفه عن العمل، لذلك لن يرتّبه نضيد.", kind.reason())
            } else {
                format!("This folder belongs to a {}. Moving its files could stop it from working, so Array will not organize it.", kind.reason())
            }),
        }
    }

    /// هل كل ملفات البرامج في جذر المجلد مثبّتات (.exe/.msi)؟ عندها المجلد ليس برنامجاً
    /// بل مجلد تنزيلات، والمثبّتات تُنقل إلى مجلد «مثبّتات».
    /// ملف مكتبة أو سكريبت أو إعدادات واحد يجعل الجواب false (تبقى الحماية).
    fn only_installers(dir: &Path) -> bool {
        let Ok(entries) = fs::read_dir(dir) else { return false };
        let mut saw_installer = false;
        for entry in entries.flatten() {
            if entry.metadata().is_ok_and(|m| Self::is_hidden_or_system(&m) || m.is_dir()) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_lowercase();
            let Some(ext) = Path::new(&name).extension().map(|e| e.to_string_lossy().to_string()) else {
                continue;
            };
            if ext == "exe" || ext == "msi" {
                saw_installer = true;
                continue;
            }
            if PROJECT_MARKERS.contains(&name.as_str())
                || PROJECT_EXTENSIONS.contains(&ext.as_str())
                || LIBRARY_EXTENSIONS.contains(&ext.as_str())
                || SCRIPT_EXTENSIONS.contains(&ext.as_str())
                || CONFIG_EXTENSIONS.contains(&ext.as_str())
                || PROGRAM_EXTENSIONS.contains(&ext.as_str())
            {
                return false;
            }
        }
        saw_installer
    }

    /// مزوّد السحابة إن كان المجلد داخل مجلد مزامنة: النقل فيه يعني رفعًا وتنزيلًا
    pub fn cloud_provider(path: &Path) -> Option<&'static str> {
        for var in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
            if let Some(dir) = std::env::var_os(var) {
                if !dir.is_empty() && path.starts_with(&dir) {
                    return Some("OneDrive");
                }
            }
        }
        let lower = path.to_string_lossy().to_lowercase();
        if lower.contains("onedrive") {
            Some("OneDrive")
        } else if lower.contains("google drive") || lower.contains("googledrive") || lower.contains("my drive") {
            Some("Google Drive")
        } else if lower.contains("dropbox") {
            Some("Dropbox")
        } else if lower.contains("icloud drive") {
            Some("iCloud Drive")
        } else {
            None
        }
    }

    /// صورة غلاف ألبوم يضعها ويندوز بجانب الموسيقى (Folder.jpg، AlbumArtSmall.jpg،
    /// AlbumArt_{GUID}_Large.jpg): تبقى في مكانها حتى لو فقدت صفة الإخفاء بالنسخ
    pub fn is_album_art(name: &str) -> bool {
        let lower = name.to_lowercase();
        lower == "folder.jpg"
            || lower == "albumartsmall.jpg"
            || (lower.starts_with("albumart_{") && (lower.ends_with("}_large.jpg") || lower.ends_with("}_small.jpg")))
    }

    /// هل يجب تخطّي هذا المجلد الفرعي بالاسم (مخفي أو مجلد نظام/برامج)؟
    pub fn is_skipped_dir_name(name: &str) -> bool {
        let lower = name.to_lowercase();
        lower.starts_with('.') || SKIPPED_DIRS.contains(&lower.as_str())
    }

    /// ملف أو مجلد يعتبره ويندوز مخفياً أو ملف نظام، أو رابط (junction/symlink) لمكان آخر
    pub fn is_hidden_or_system(metadata: &fs::Metadata) -> bool {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM | FILE_ATTRIBUTE_REPARSE_POINT) != 0
        }
        #[cfg(not(windows))]
        {
            metadata.file_type().is_symlink()
        }
    }

    /// هل المجلد يخص برنامجاً أو أداة أو مشروعاً؟ يُفحص محتواه المباشر فقط.
    /// نقل أي ملف من مثل هذا المجلد قد يوقف البرنامج، لذلك يُترك المجلد كله.
    pub fn program_folder(dir: &Path) -> Option<ProgramFolder> {
        let entries = fs::read_dir(dir).ok()?;
        let mut found: Option<ProgramFolder> = None;

        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            // علامات المشاريع تُفحص بالاسم حتى لو كانت مخفية (مثل .git)
            if PROJECT_MARKERS.contains(&name.as_str()) {
                return Some(ProgramFolder::Project(entry.file_name().to_string_lossy().to_string()));
            }
            // ملفات ويندوز المخفية مثل desktop.ini توجد في كل مجلد ولا تعني شيئاً
            if entry.metadata().is_ok_and(|m| Self::is_hidden_or_system(&m) || m.is_dir()) {
                continue;
            }
            let Some(ext) = Path::new(&name).extension().map(|e| e.to_string_lossy().to_string()) else {
                continue;
            };
            let original = entry.file_name().to_string_lossy().to_string();
            let kind = if PROJECT_EXTENSIONS.contains(&ext.as_str()) {
                return Some(ProgramFolder::Project(original));
            } else if LIBRARY_EXTENSIONS.contains(&ext.as_str()) {
                ProgramFolder::Library(original)
            } else if SCRIPT_EXTENSIONS.contains(&ext.as_str()) {
                ProgramFolder::Script(original)
            } else if PROGRAM_EXTENSIONS.contains(&ext.as_str()) {
                ProgramFolder::Program(original)
            } else if CONFIG_EXTENSIONS.contains(&ext.as_str()) {
                ProgramFolder::Config(original)
            } else {
                continue;
            };
            // الأخطر أولاً: مكتبة > سكريبت > برنامج > إعدادات
            if found.as_ref().is_none_or(|f| Self::rank(&kind) < Self::rank(f)) {
                found = Some(kind);
            }
        }
        found
    }

    fn rank(kind: &ProgramFolder) -> u8 {
        match kind {
            ProgramFolder::Project(_) => 0,
            ProgramFolder::Library(_) => 1,
            ProgramFolder::Script(_) => 2,
            ProgramFolder::Program(_) => 3,
            ProgramFolder::Config(_) => 4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::organizer::tests::temp_tree;

    #[test]
    fn drive_roots_and_system_folders_are_blocked() {
        for p in [
            "C:\\",
            "D:\\",
            "C:\\Windows\\System32",
            "C:\\Program Files\\App",
            "C:\\Program Files (x86)\\App",
            "C:\\ProgramData",
            "C:\\Users",
            "C:\\Users\\me\\AppData\\Roaming\\App",
            "D:\\Recovery",
        ] {
            assert!(!SafetyChecker::is_safe_root(Path::new(p)), "{p} should be blocked");
        }
    }

    #[test]
    fn user_folders_are_allowed() {
        assert!(SafetyChecker::is_safe_root(Path::new("C:\\Users\\me\\Downloads")));
        assert!(SafetyChecker::is_safe_root(Path::new("C:\\Users\\me")));
        assert!(SafetyChecker::is_safe_root(Path::new("D:\\Music")));
        // كلمة windows داخل اسم مجلد عادي لا تمنعه
        assert!(SafetyChecker::is_safe_root(Path::new("C:\\Users\\me\\Downloads\\windows-wallpapers")));
    }

    #[test]
    fn cloud_folders_are_recognized() {
        assert_eq!(SafetyChecker::cloud_provider(Path::new("C:\\Users\\me\\OneDrive\\Pictures")), Some("OneDrive"));
        assert_eq!(SafetyChecker::cloud_provider(Path::new("D:\\Google Drive\\Photos")), Some("Google Drive"));
        assert_eq!(SafetyChecker::cloud_provider(Path::new("C:\\Users\\me\\Dropbox\\Camera Uploads")), Some("Dropbox"));
        assert_eq!(SafetyChecker::cloud_provider(Path::new("C:\\Users\\me\\Downloads")), None);
    }

    #[test]
    fn hidden_and_app_folders_are_skipped() {
        assert!(SafetyChecker::is_skipped_dir_name(".git"));
        assert!(SafetyChecker::is_skipped_dir_name("AppData"));
        assert!(SafetyChecker::is_skipped_dir_name("node_modules"));
        assert!(!SafetyChecker::is_skipped_dir_name("Downloads"));
    }

    #[test]
    fn detects_program_and_project_folders() {
        let root = temp_tree("detect", &[
            "photos/a.jpg",
            "photos/b.png",
            "tool/tool.exe",
            "tool/icon.png",
            "app/core.dll",
            "proj/Cargo.toml",
            "repo/.git/HEAD",
            "scripts/backup.ps1",
            "site/page.html",
            "game/settings.ini",
            "vs/App.sln",
        ]);
        let kind = |d: &str| SafetyChecker::program_folder(&root.join(d));

        assert_eq!(kind("photos"), None);
        assert!(matches!(kind("tool"), Some(ProgramFolder::Program(_))));
        assert!(matches!(kind("app"), Some(ProgramFolder::Library(_))));
        assert!(matches!(kind("proj"), Some(ProgramFolder::Project(_))));
        assert!(matches!(kind("repo"), Some(ProgramFolder::Project(_))));
        assert!(matches!(kind("scripts"), Some(ProgramFolder::Script(_))));
        assert!(matches!(kind("site"), Some(ProgramFolder::Script(_))));
        assert!(matches!(kind("game"), Some(ProgramFolder::Config(_))));
        assert!(matches!(kind("vs"), Some(ProgramFolder::Project(_))));
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn hidden_desktop_ini_does_not_mark_a_folder() {
        let root = temp_tree("desktop_ini", &["Pictures/desktop.ini", "Pictures/a.jpg"]);
        let ini = root.join("Pictures/desktop.ini");
        std::process::Command::new("attrib").arg("+h").arg("+s").arg(&ini).status().unwrap();

        assert_eq!(SafetyChecker::program_folder(&root.join("Pictures")), None);
        std::process::Command::new("attrib").arg("-h").arg("-s").arg(&ini).status().unwrap();
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn selected_folder_is_refused_when_it_is_a_program() {
        let root = temp_tree("root_check", &[
            "MyTool/tool.exe",
            "MyTool/data.txt",
            "Downloads/setup.exe",
            "Downloads/movie.mp4",
            "Desktop/backup.bat",
            "Project/package.json",
        ]);

        // مجلد فيه مثبّت واحد فقط لم يعد يُرفض: المثبّت فئة تُنقل إلى «مثبّتات» (بند 17)
        assert!(SafetyChecker::check_root(&root.join("MyTool")).is_ok());
        assert!(SafetyChecker::check_root(&root.join("Project")).is_err());
        // سكريبت على سطح المكتب قد يعتمد على الملفات المجاورة له
        assert!(SafetyChecker::check_root(&root.join("Desktop")).is_err());
        // ملف تثبيت في Downloads لا يمنع تنظيمها
        assert!(SafetyChecker::check_root(&root.join("Downloads")).is_ok());
        // مكتبة أو سكريبت أو كود مع المثبّت تُبقي الحماية
        let guarded = temp_tree("root_guarded", &["App/app.exe", "App/core.dll", "Site/page.html", "Hold/setup.exe", "Hold/install.bat"]);
        assert!(SafetyChecker::check_root(&guarded.join("App")).is_err());
        assert!(SafetyChecker::check_root(&guarded.join("Site")).is_err());
        assert!(SafetyChecker::check_root(&guarded.join("Hold")).is_err());
        fs::remove_dir_all(&guarded).unwrap();
        fs::remove_dir_all(&root).unwrap();
    }
}
