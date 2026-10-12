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
            Some(kind) => Err(if crate::i18n::ar() {
                format!("هذا المجلد فيه {}. نقل ملفاته قد يوقفه عن العمل، لذلك لن يرتّبه Array.", kind.reason())
            } else {
                format!("This folder belongs to a {}. Moving its files could stop it from working, so Array will not organize it.", kind.reason())
            }),
        }
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

        assert!(SafetyChecker::check_root(&root.join("MyTool")).is_err());
        assert!(SafetyChecker::check_root(&root.join("Project")).is_err());
        // سكريبت على سطح المكتب قد يعتمد على الملفات المجاورة له
        assert!(SafetyChecker::check_root(&root.join("Desktop")).is_err());
        // ملف تثبيت في Downloads لا يمنع تنظيمها
        assert!(SafetyChecker::check_root(&root.join("Downloads")).is_ok());
        fs::remove_dir_all(&root).unwrap();
    }
}
