#![windows_subsystem = "windows"]

mod grouping;
mod i18n;
mod icons;
mod naming;
mod normalizer;
mod organizer;
mod rtl;
mod safety;
mod storage;
mod theme;
mod video;

use chrono::{DateTime, Local};
use eframe::egui::{self, RichText};

use i18n::{ar, count, tr, Noun};
use icons::Icon;
use organizer::{Journal, Options, OrganizerEngine, Plan, RunResult, Summary};
use naming::NamingGroup;
use safety::SafetyChecker;
use rfd::FileDialog;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use storage::{Storage, UndoLog};
use theme::{pal, t, Kind};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> eframe::Result<()> {
    let icon = load_app_icon();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Array")
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([900.0, 620.0])
            .with_drag_and_drop(true)
            .with_icon(Arc::new(icon)),
        // السمة يديرها التطبيق نفسه (تتبع ويندوز أو اختيار المستخدم)
        follow_system_theme: false,
        ..Default::default()
    };

    eframe::run_native(
        "Array",
        options,
        Box::new(|cc| {
            theme::install_fonts(&cc.egui_ctx);
            Box::new(ArrayApp::new(&cc.egui_ctx, Storage::default_location()))
        }),
    )
}

/// أيقونة النافذة وشريط المهام من نفس ملف assets/app.ico المضمّن في ملف EXE
fn load_app_icon() -> egui::IconData {
    match image::load_from_memory_with_format(include_bytes!("../assets/app.ico"), image::ImageFormat::Ico) {
        Ok(img) => {
            let rgba = img.into_rgba8();
            let (width, height) = rgba.dimensions();
            egui::IconData { rgba: rgba.into_raw(), width, height }
        }
        Err(_) => egui::IconData::default(),
    }
}

/// شعار التطبيق داخل الواجهة (نفس الأيقونة بحجم 256)
fn load_logo(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let img = image::load_from_memory(include_bytes!("../assets/app-256.png")).ok()?.into_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture("logo", color, egui::TextureOptions::LINEAR))
}

/// عمل جارٍ في الخلفية حتى لا تتجمد النافذة مع المجلدات الكبيرة
enum Job {
    Planning(Receiver<Result<Plan, String>>),
    /// تنظيم أو تراجع، مع عدّاد الملفات المنجزة لشريط التقدم
    Moving {
        restoring: bool,
        rx: Receiver<Result<String, String>>,
        progress: Arc<AtomicUsize>,
        total: usize,
        folder: PathBuf,
    },
}

fn spawn_job<T: Send + 'static>(ctx: &egui::Context, work: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    thread::spawn(move || {
        let _ = tx.send(work());
        ctx.request_repaint();
    });
    rx
}

/// سجل التراجع جاهزًا للتنفيذ: لو نُقل مجلد التنظيم بعد الترتيب تُبنى مساراته إلى مكانه الحالي
fn undo_log_for(storage: &Storage, folder: Option<&Path>) -> Result<Option<UndoLog>, String> {
    let Some(log) = storage.load_undo()? else {
        return Ok(None);
    };
    // يُستعمل المكان المختار فقط إذا كان يحمل بيان المجلد نفسه، فلا تُخمَّن المسارات
    let moved_to = folder.filter(|p| !log.source.exists() && Journal::load(p).is_some_and(|j| j.source == log.source));
    Ok(Some(match moved_to {
        Some(current) => log.rebased(current),
        None => log,
    }))
}

/// (سجلات التراجع، عدد ما يمكن إرجاعه، مجلد التراجع): سجل AppData أولًا ثم بيان المجلد المختار
fn undo_state(storage: &Storage, selected: Option<&Path>) -> Result<(usize, usize, Option<PathBuf>), String> {
    if let Some(log) = undo_log_for(storage, selected)? {
        let movable = log.moves.iter().filter(|m| m.to.exists()).count();
        return Ok((log.moves.len(), movable, Some(log.source)));
    }
    let Some((folder, journal)) = selected
        .filter(|p| OrganizerEngine::has_journal(p))
        .and_then(|p| Journal::load(p).map(|j| (p, j)))
    else {
        return Ok((0, 0, None));
    };
    let movable = journal.restorable(folder);
    Ok((movable, movable, Some(folder.to_path_buf())))
}

/// تراجع: بيان المجلد أولًا (يعمل بلا سجل AppData)، وإن كان تالفًا أو بائتًا فسجل AppData.
fn undo_folder(folder: &Path, storage: &Storage, counter: &AtomicUsize) -> Result<RunResult, String> {
    let has_journal = OrganizerEngine::has_journal(folder);
    if let Some(result) = has_journal.then(|| OrganizerEngine::undo_from_journal(folder, counter)).flatten() {
        // بيان بائت (لا مدخل done فيه) يُنجز صفرًا: لا يُمسح السجل الصحيح، ويُجرَّب بعده
        if !result.done.is_empty() || !result.failed.is_empty() {
            if result.failed.is_empty() {
                storage.clear_undo()?;
            }
            return Ok(result);
        }
    }
    let Some(log) = undo_log_for(storage, Some(folder))? else {
        return Err(if has_journal {
            tr("The record in this folder is damaged.", "البيان في هذا المجلد تالف.").to_string()
        } else {
            tr("There is nothing to undo.", "لا يوجد ما يمكن التراجع عنه.").to_string()
        });
    };
    let result = OrganizerEngine::undo(&log.moves, &log.source, &log.created_dirs, counter);
    // الملفات التي لم تُرجع تبقى في السجل لإعادة المحاولة
    if result.failed.is_empty() {
        storage.clear_undo()?;
    } else {
        storage.save_undo(&UndoLog { source: log.source.clone(), moves: result.failed.clone(), created_dirs: log.created_dirs.clone() })?;
    }
    Ok(result)
}

/// قراءة نتيجة العمل إن انتهى؛ توقف الخيط بشكل غير متوقع يُعامل كخطأ
fn poll<T>(rx: &Receiver<Result<T, String>>) -> Option<Result<T, String>> {
    match rx.try_recv() {
        Ok(result) => Some(result),
        Err(TryRecvError::Disconnected) => Some(Err(tr("The operation stopped unexpectedly.", "توقفت العملية بشكل غير متوقع.").to_string())),
        Err(TryRecvError::Empty) => None,
    }
}

/// المجلد المختار نفسه في ملخص المعاينة
const THIS_FOLDER: &str = "\u{1}this-folder";

/// خطة جاهزة للعرض. النصوص تُحفظ بترتيبها المنطقي وتُجهّز للرسم عند العرض
/// ما تعرضه شاشة النتيجة: نُقل، أُعيد تسميته، مكررات، وتُرك مع السبب
#[derive(Clone, Copy, Default)]
struct RunStats {
    moved: usize,
    renamed: usize,
    duplicates: usize,
    left: usize,
}

struct Preview {
    plan: Plan,
    /// (اسم المجلد، عدد الملفات، موجود أم جديد، عدد مجلدات الفنانين)
    destinations: Vec<(String, usize, bool, usize)>,
    /// (المكان الحالي، المكان الجديد)
    rows: Vec<(String, String)>,
    /// مجلدات برامج ومشاريع تُركت كما هي، مع السبب
    skipped: Vec<String>,
}

impl Preview {
    fn new(plan: Plan) -> Self {
        let relative = |path: &Path, base: &Path| path.strip_prefix(base).unwrap_or(path).display().to_string();

        // عدد الملفات لكل مجلد وجهة، وعدد مجلدات الفنانين داخل مجلد الموسيقى
        let mut per_folder: BTreeMap<String, usize> = BTreeMap::new();
        let mut artist_folders: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for m in &plan.moves {
            let parts: Vec<String> = m
                .to
                .strip_prefix(&plan.source)
                .unwrap_or(&m.to)
                .components()
                .filter_map(|c| match c {
                    Component::Normal(n) => Some(n.to_string_lossy().to_string()),
                    _ => None,
                })
                .collect();
            // ملف يبقى في المجلد المختار نفسه (يُعاد تسميته فقط)
            let folder = if parts.len() > 1 { parts[0].clone() } else { THIS_FOLDER.to_string() };
            *per_folder.entry(folder.clone()).or_default() += 1;
            if parts.len() > 2 {
                artist_folders.entry(folder).or_default().insert(parts[1].clone());
            }
        }
        let mut destinations: Vec<(String, usize, bool, usize)> = per_folder
            .into_iter()
            .map(|(folder, n)| {
                let existing = folder == THIS_FOLDER || !plan.new_dirs.contains(&plan.source.join(&folder));
                let artists = artist_folders.get(&folder).map_or(0, |s| s.len());
                (folder, n, existing, artists)
            })
            .collect();
        destinations.sort_by(|a, b| b.1.cmp(&a.1));

        let rows = plan
            .moves
            .iter()
            .map(|m| (relative(&m.from, &plan.source), relative(&m.to, &plan.source)))
            .collect();

        let skipped = plan.skipped.iter().map(|(dir, reason)| format!("{}  —  {}", relative(dir, &plan.source), reason)).collect();

        Self { plan, destinations, rows, skipped }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Organize,
    History,
    Settings,
    About,
}

/// نتيجة آخر عملية لعرضها في بطاقة النتيجة
struct Outcome {
    message: String,
    folder: PathBuf,
    ok: bool,
}

struct ArrayApp {
    storage: Storage,
    options: Options,
    logo: Option<egui::TextureHandle>,
    page: Page,
    selected_path: Option<PathBuf>,
    status_message: String,
    job: Option<Job>,
    preview: Option<Preview>,
    outcome: Option<Outcome>,
    /// عدد الملفات التي يمكن إرجاعها (None إذا لا يوجد ما يُتراجع عنه)
    undo_count: Option<usize>,
    /// عدد السجلات في ملف التراجع (لشريط التقدم)
    undo_entries: usize,
    undo_source: Option<PathBuf>,
    confirm_undo: bool,
    show_skipped: bool,
    /// الجدول التفصيلي بدل بطاقات الملخص
    show_all: bool,
    /// إحصاءات آخر تنفيذ (شاشة النتيجة)
    last_stats: Option<RunStats>,
    /// السمة المطبّقة حالياً (None قبل أول إطار)
    dark: Option<bool>,
    /// فحص «مكررات للمراجعة» لزر النقل إلى سلة المحذوفات
    review_scan: Option<organizer::DuplicateScan>,
    /// مجلد العملية التي فُتح تأكيد نقل مكرراتها
    trash_folder: Option<PathBuf>,
    confirm_trash: bool,
}

fn ready_message() -> String {
    tr("Choose a folder to start.", "اختر مجلداً للبدء.").to_string()
}

impl ArrayApp {
    fn new(ctx: &egui::Context, storage: Storage) -> Self {
        let loaded = storage.load_settings();
        let options = loaded.as_ref().map(|o| o.clone()).unwrap_or_default();
        i18n::set_arabic(options.language != "en");
        let status_message = match loaded {
            Ok(_) => ready_message(),
            Err(err) => format!("⚠ {}", err),
        };
        let mut app = Self {
            storage,
            options,
            logo: load_logo(ctx),
            page: Page::Organize,
            selected_path: None,
            status_message,
            job: None,
            preview: None,
            outcome: None,
            undo_count: None,
            undo_entries: 0,
            undo_source: None,
            confirm_undo: false,
            show_skipped: false,
            show_all: false,
            last_stats: None,
            dark: None,
            review_scan: None,
            trash_folder: None,
            confirm_trash: false,
        };
        app.refresh_undo();
        app
    }

    fn refresh_undo(&mut self) {
        // بعد تنظيم توقف في المنتصف يحتوي السجل ملفات لم تُنقل، فلا تُحسب
        match undo_state(&self.storage, self.selected_path.as_deref()) {
            Ok((entries, movable, source)) => {
                self.undo_entries = entries;
                self.undo_count = Some(movable).filter(|&n| n > 0);
                self.undo_source = source;
            }
            Err(err) => {
                self.status_message = format!("⚠ {}", err);
                self.undo_entries = 0;
                self.undo_count = None;
                self.undo_source = None;
            }
        }
    }

    fn busy(&self) -> bool {
        self.job.is_some() || self.preview.is_some() || self.confirm_undo
    }

    fn poll_job(&mut self) {
        let Some(job) = &self.job else { return };
        match job {
            Job::Planning(rx) => match poll(rx) {
                None => return,
                Some(Ok(plan)) if plan.moves.is_empty() => {
                    let mut message = if plan.kept_in_folders > 0 {
                        if ar() {
                            format!("لا شيء للترتيب: الملفات المتفرقة في مكانها، و{} في مجلداتك تبقى كما هي.", count(plan.kept_in_folders, Noun::File))
                        } else {
                            format!(
                                "Nothing to organize: the loose files are already in place and {} in your existing folders are left as they are.",
                                count(plan.kept_in_folders, Noun::File)
                            )
                        }
                    } else if plan.in_place > 0 {
                        if ar() {
                            format!("هذا المجلد مرتّب أصلاً ({} في مكانها الصحيح).", count(plan.in_place, Noun::File))
                        } else {
                            format!("This folder is already organized ({} in the right place).", count(plan.in_place, Noun::File))
                        }
                    } else {
                        tr("No files to organize in this folder.", "لا توجد ملفات للترتيب في هذا المجلد.").to_string()
                    };
                    if !plan.skipped.is_empty() {
                        message.push_str(&if ar() {
                            format!(" وتُرك {} دون لمس.", count(plan.skipped.len(), Noun::ProtectedFolder))
                        } else {
                            format!(" {} left untouched.", count(plan.skipped.len(), Noun::ProtectedFolder))
                        });
                    }
                    self.status_message = message.clone();
                    self.outcome = Some(Outcome { message, folder: plan.source.clone(), ok: true });
                }
                Some(Ok(plan)) => {
                    self.status_message = tr("Review where each file will go, then confirm.", "راجع إلى أين سيذهب كل ملف، ثم أكّد.").to_string();
                    self.show_skipped = false;
                    self.show_all = false;
                    self.preview = Some(Preview::new(plan));
                }
                Some(Err(err)) => {
                    self.status_message = format!("❌ {}", err);
                    let folder = self.selected_path.clone().unwrap_or_default();
                    self.outcome = Some(Outcome { message: err, folder, ok: false });
                }
            },
            Job::Moving { rx, folder, .. } => {
                let folder = folder.clone();
                match poll(rx) {
                    None => return,
                    Some(Ok(msg)) => {
                        self.status_message = msg.lines().next().unwrap_or_default().to_string();
                        self.outcome = Some(Outcome { message: msg, folder, ok: true });
                    }
                    Some(Err(err)) => {
                        self.status_message = format!("❌ {}", err);
                        self.outcome = Some(Outcome { message: err, folder, ok: false });
                    }
                }
            }
        }
        self.job = None;
        self.refresh_undo();
    }

    /// قراءة الإعدادات من جديد لأن المستخدم قد يكون عدّلها في Notepad
    fn reload_settings(&mut self) {
        match self.storage.load_settings() {
            Ok(options) => self.options = options,
            Err(err) => self.status_message = format!("⚠ {}", err),
        }
    }

    fn select_folder(&mut self, path: PathBuf) {
        self.selected_path = Some(path);
        // مجلد فيه بيان تنظيم سابق: زر التراجع متاح حتى لو حُذف سجل AppData
        self.refresh_undo();
        self.outcome = None;
        self.status_message = tr("Ready. Press Preview to see where each file will go.", "جاهز. اضغط معاينة لترى أين سيذهب كل ملف.").to_string();
    }

    fn start_planning(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.reload_settings();
        self.outcome = None;
        let options = self.options.clone();
        self.job = Some(Job::Planning(spawn_job(ctx, move || {
            OrganizerEngine::plan_with(&path, &options)
        })));
        self.status_message = format!("⏳ {}", tr("Scanning files…", "جارٍ فحص الملفات…"));
    }

    fn start_organizing(&mut self, ctx: &egui::Context, plan: Plan) {
        // الإحصاءات من الخطة نفسها قبل نقل أي ملف
        let mut stats = RunStats { duplicates: plan.duplicates, left: plan.left.len(), ..RunStats::default() };
        for m in &plan.moves {
            // الملف قد يُنقل ويُعاد تسميته في العملية نفسها، فيُحسب في الاثنين
            if m.from.parent() != m.to.parent() {
                stats.moved += 1;
            }
            if m.from.file_name() != m.to.file_name() {
                stats.renamed += 1;
            }
        }
        self.last_stats = Some(stats);
        let progress = Arc::new(AtomicUsize::new(0));
        let total = plan.moves.len();
        let folder = plan.source.clone();
        let storage = self.storage.clone();
        let counter = progress.clone();
        let rx = spawn_job(ctx, move || {
            // السجل يُحفظ قبل نقل أي ملف؛ إن تعذّر حفظه لا يُنقل شيء
            storage
                .save_undo(&UndoLog { source: plan.source.clone(), moves: plan.moves.clone(), created_dirs: plan.new_dirs.clone() })
                .map_err(|e| format!("{} {}", e, tr("Nothing was moved.", "لم يُنقل أي ملف.")))?;
            let result = OrganizerEngine::execute(&plan, &counter);
            // السجل يحتفظ فقط بما نُقل فعلاً
            let saved = if result.done.is_empty() {
                storage.clear_undo()
            } else {
                storage.save_undo(&UndoLog { source: plan.source.clone(), moves: result.done.clone(), created_dirs: plan.new_dirs.clone() })
            };

            let done = count(result.done.len(), Noun::File);
            let mut msg = if ar() {
                format!("✔ رُتّب {} في {}", done, folder_name(&plan.source))
            } else {
                format!("✔ Organized {} in {}", done, folder_name(&plan.source))
            };
            if !result.failed.is_empty() {
                let failed = count(result.failed.len(), Noun::File);
                msg.push_str(&if ar() {
                    format!("\n⚠ تعذّر نقل {} (مفتوح في برنامج آخر، أو بلا صلاحية، أو ظهر ملف بنفس الاسم) فبقي في مكانه.", failed)
                } else {
                    format!("\n⚠ {} could not be moved (in use, access denied, or a file with the same name appeared) and were left in place.", failed)
                });
            }
            if let Some(warning) = &result.warning {
                msg.push_str(&if ar() {
                    format!("\n⚠ رُتّبت الملفات لكن تعذّر كتابة بيانها داخل المجلد ({}). التراجع متاح من سجل البرنامج.", warning)
                } else {
                    format!("\n⚠ Files were organized, but their record inside the folder could not be written ({}). Undo is still available from the app's log.", warning)
                });
            }
            if let Err(e) = saved {
                msg.push_str(&format!("\n⚠ {}", e));
            }
            Ok(msg)
        });
        self.job = Some(Job::Moving { restoring: false, rx, progress, total, folder });
        self.status_message = format!("⏳ {}", tr("Organizing… please wait", "جارٍ الترتيب… لحظات"));
    }

    fn start_undo(&mut self, ctx: &egui::Context) {
        let progress = Arc::new(AtomicUsize::new(0));
        let total = self.undo_entries;
        let folder = self.undo_source.clone().unwrap_or_default();
        let job_folder = folder.clone();
        let storage = self.storage.clone();
        let counter = progress.clone();
        let rx = spawn_job(ctx, move || {
            let result = undo_folder(&folder, &storage, &counter)?;

            let done = count(result.done.len(), Noun::File);
            let mut msg = if ar() {
                format!("↩ أُعيد {} إلى أماكنها الأصلية في {}", done, folder_name(&folder))
            } else {
                format!("↩ Restored {} to their original places in {}", done, folder_name(&folder))
            };
            if !result.failed.is_empty() {
                let failed = count(result.failed.len(), Noun::File);
                msg.push_str(&if ar() {
                    format!("\n⚠ تعذّرت إعادة {} (نُقل أو أُعيدت تسميته أو مفتوح أو يوجد ملف جديد بنفس الاسم). اضغط تراجع مرة أخرى لإعادة المحاولة.", failed)
                } else {
                    format!("\n⚠ {} could not be restored (moved, renamed, in use, or a new file has the same name). Press Undo again to retry them.", failed)
                });
            }
            if let Some(warning) = &result.warning {
                msg.push_str(&if ar() {
                    format!("\n⚠ أُعيدت الملفات لكن تعذّر تحديث بيان المجلد ({}).", warning)
                } else {
                    format!("\n⚠ Files were restored, but the folder's record could not be updated ({}).", warning)
                });
            }
            Ok(msg)
        });
        self.last_stats = None;
        self.job = Some(Job::Moving { restoring: true, rx, progress, total, folder: job_folder });
        self.page = Page::Organize;
        self.status_message = format!("⏳ {}", tr("Restoring files… please wait", "جارٍ إعادة الملفات… لحظات"));
    }

    fn set_option(&mut self, change: impl Fn(&mut Options)) {
        // قراءة الملف ثم تعديل هذا الخيار فقط، حتى لا تضيع تعديلات المستخدم على الكلمات
        match self.storage.load_settings() {
            Ok(mut options) => {
                change(&mut options);
                if let Err(err) = self.storage.save_settings(&options) {
                    self.status_message = format!("⚠ {}", err);
                }
                self.options = options;
            }
            Err(err) => {
                change(&mut self.options);
                self.status_message = format!("⚠ {} {}", err, tr("This choice is used for now but not saved.", "يُستخدم هذا الاختيار الآن لكنه لم يُحفظ."));
            }
        }
    }

    fn set_language(&mut self, arabic: bool) {
        let value = if arabic { "ar" } else { "en" }.to_string();
        self.set_option(move |o| o.language = value.clone());
        i18n::set_arabic(arabic);
        // الرسائل السابقة كُتبت باللغة القديمة
        self.status_message = ready_message();
        self.outcome = None;
    }

    fn open_settings_file(&mut self) {
        let path = self.storage.settings_path();
        if !path.exists() {
            if let Err(err) = self.storage.save_settings(&self.options) {
                self.status_message = format!("⚠ {}", err);
                return;
            }
        }
        self.status_message = match std::process::Command::new("notepad").arg(&path).spawn() {
            Ok(_) => tr("Edit the lists in Notepad and save. They are used from the next preview.", "عدّل القوائم في Notepad واحفظ. تُستخدم من المعاينة التالية.").to_string(),
            Err(err) => format!("⚠ {} {}: {}", tr("Could not open", "تعذّر فتح"), path.display(), err),
        };
    }

    fn open_in_explorer(&mut self, path: &Path) {
        if let Err(err) = std::process::Command::new("explorer").arg(path).spawn() {
            self.status_message = format!("⚠ {} {}: {}", tr("Could not open", "تعذّر فتح"), path.display(), err);
        }
    }

    // ---------------- الواجهة ----------------

    fn sidebar(&mut self, ctx: &egui::Context) {
        let p = pal();
        let panel = if ar() { egui::SidePanel::right("nav") } else { egui::SidePanel::left("nav") };
        panel
            .exact_width(228.0)
            .resizable(false)
            // الهوامش داخل إطار ثانٍ: egui يجعل عرض المحتوى نفسه 228 فيتجاوز الإطار حافة النافذة اليمنى
            .frame(egui::Frame::none().fill(p.sidebar))
            .show(ctx, |ui| {
                egui::Frame::none().inner_margin(egui::Margin::symmetric(14.0, 22.0)).show(ui, |ui| self.sidebar_contents(ui));
            });
    }

    fn sidebar_contents(&mut self, ui: &mut egui::Ui) {
        let p = pal();
        theme::row(ui, |ui| {
            ui.add_space(4.0);
            if let Some(logo) = &self.logo {
                ui.add(egui::Image::from_texture(egui::load::SizedTexture::new(logo.id(), [42.0, 42.0])));
            }
            theme::column(ui, |ui| {
                ui.label(theme::strong("Array", 21.0));
                ui.label(theme::muted(t("File organizer", "منظّم الملفات")).small());
            });
        });
        ui.add_space(34.0);

        let busy = self.busy();
        for (page, icon, en, arabic) in [
            (Page::Organize, Icon::Organize, "Organize", "التنظيم"),
            (Page::History, Icon::History, "History", "السجل"),
            (Page::Settings, Icon::Settings, "Settings", "الإعدادات"),
            (Page::About, Icon::About, "About", "حول"),
        ] {
            // أثناء المعاينة أو العمل تبقى صفحة التنظيم ظاهرة
            if theme::nav_item(ui, icon, tr(en, arabic), self.page == page, !busy || page == Page::Organize).clicked() {
                self.page = page;
            }
            ui.add_space(2.0);
        }

        ui.with_layout(egui::Layout::bottom_up(if ar() { egui::Align::Max } else { egui::Align::Min }), |ui| {
            ui.label(theme::muted(t(&format!("Version {}", VERSION), &format!("الإصدار {}", VERSION))).small());
            theme::icon_text(ui, Icon::Lock, theme::muted(t("Works offline on this PC", "يعمل دون إنترنت على جهازك")).small(), p.muted);
        });
    }

    fn page_header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
        ui.add_space(4.0);
        ui.label(theme::strong(theme::show(title), 28.0));
        theme::para(ui, subtitle, 15.0, pal().muted, false);
        ui.add_space(16.0);
    }

    fn organize_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.preview.is_some() {
            self.preview_view(ui, ctx);
            return;
        }
        if self.job.is_some() {
            Self::page_header(ui, tr("Organizing", "جارٍ الترتيب"), tr("Working on it — you can keep using your computer.", "العمل جارٍ — يمكنك متابعة استخدام جهازك."));
            self.progress_view(ui);
            return;
        }

        Self::page_header(
            ui,
            tr("Organize a folder", "رتّب مجلداً"),
            tr(
                "Sort loose photos, music, videos and documents into tidy folders. Nothing moves until you confirm.",
                "يرتّب الصور والموسيقى والفيديو والمستندات المتفرقة في مجلدات مرتبة. لا يتحرك أي ملف قبل موافقتك.",
            ),
        );
        let p = pal();

        egui::ScrollArea::vertical().auto_shrink([false, false]).max_height((ui.available_height() - 60.0).max(120.0)).show(ui, |ui| {
            theme::column(ui, |ui| {
                if let Some(outcome) = self.outcome.take() {
                    if self.outcome_view(ui, &outcome) {
                        self.outcome = Some(outcome);
                    }
                    ui.add_space(14.0);
                }

                // الخطوة 1: المجلد
                theme::card(ui, |ui| {
                    theme::step_title(ui, "1", &t("Choose a folder", "اختر مجلداً"));
                    ui.add_space(10.0);
                    let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
                    egui::Frame::none()
                        .fill(if hovering { p.selected } else { p.inset })
                        .stroke(egui::Stroke::new(if hovering { 2.0_f32 } else { 1.0_f32 }, if hovering { p.accent } else { p.border }))
                        .rounding(theme::RADIUS)
                        .inner_margin(egui::Margin::same(20.0))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            match self.selected_path.clone() {
                                Some(path) => {
                                    theme::column(ui, |ui| {
                                        theme::row(ui, |ui| {
                                            icons::show(ui, Icon::Folder, 34.0, p.accent);
                                            ui.label(theme::strong(rtl::for_display(&folder_name(&path)), 18.0));
                                        });
                                        ui.label(theme::muted(rtl::for_display(&path.display().to_string())).small());
                                        ui.add_space(10.0);
                                        theme::row(ui, |ui| {
                                            if theme::button(ui, Kind::Secondary, Some(Icon::Folder), tr("Change folder", "تغيير المجلد"), true).clicked() {
                                                if let Some(picked) = FileDialog::new().pick_folder() {
                                                    self.select_folder(picked);
                                                }
                                            }
                                            if theme::button(ui, Kind::Ghost, Some(Icon::External), tr("Open in Explorer", "فتح في المستكشف"), true).clicked() {
                                                self.open_in_explorer(&path);
                                            }
                                        });
                                    });
                                }
                                None => {
                                    ui.vertical_centered(|ui| {
                                        icons::show(ui, Icon::Drop, 40.0, if hovering { p.accent } else { p.muted });
                                        ui.add_space(4.0);
                                        let title = if hovering {
                                            t("Release to choose this folder", "أفلِت لاختيار هذا المجلد")
                                        } else {
                                            t("Drop a folder here", "اسحب مجلداً وأفلته هنا")
                                        };
                                        ui.label(theme::strong(title, 17.0));
                                        ui.label(theme::muted(t("or", "أو")));
                                        if theme::button(ui, Kind::Secondary, Some(Icon::Folder), tr("Browse…", "استعراض…"), true).clicked() {
                                            if let Some(picked) = FileDialog::new().pick_folder() {
                                                self.select_folder(picked);
                                            }
                                        }
                                    });
                                }
                            }
                        });
                });
                ui.add_space(14.0);

                // الخطوة 2: الخيارات
                theme::card(ui, |ui| {
                    theme::step_title(ui, "2", &t("How to organize", "طريقة الترتيب"));
                    ui.add_space(10.0);
                    self.option_toggles(ui, true);
                });
                ui.add_space(8.0);
            });
        });

        // الخطوة 3: زر المعاينة مثبّت أسفل الصفحة فيبقى ظاهراً دائماً
        ui.add_space(12.0);
        theme::row(ui, |ui| {
            let can_start = self.selected_path.is_some();
            if theme::button(ui, Kind::Primary, Some(Icon::Search), tr("Preview changes", "معاينة التغييرات"), can_start).clicked() {
                if let Some(path) = self.selected_path.clone() {
                    self.start_planning(ctx, path);
                }
            }
            ui.add_space(6.0);
            let hint = if can_start {
                t("You will see every change before anything moves.", "سترى كل تغيير قبل أن يتحرك أي ملف.")
            } else {
                t("Choose a folder first.", "اختر مجلداً أولاً.")
            };
            ui.label(theme::muted(hint));
        });
    }

    /// مفاتيح الخيارات الثلاثة (في صفحة التنظيم بشرح مفصّل، وفي الإعدادات مختصرة)
    fn option_toggles(&mut self, ui: &mut egui::Ui, detailed: bool) {
        let mut keep = self.options.keep_existing_folders;
        let help = if detailed {
            tr(
                "Files already inside subfolders are not moved or renamed. Turn off to empty and rebuild all subfolders.",
                "الملفات الموجودة داخل المجلدات الفرعية لا تُنقل ولا يُعاد تسميتها. أوقفه لتفريغ كل المجلدات الفرعية وإعادة بنائها.",
            )
        } else {
            tr("Only loose files are organized.", "تُرتَّب الملفات المتفرقة فقط.")
        };
        if theme::toggle_row(ui, true, &mut keep, tr("Keep my existing folders as they are", "أبقِ مجلداتي الحالية كما هي"), help) {
            self.set_option(|o| o.keep_existing_folders = keep);
        }
        ui.add_space(8.0);
        let mut by_year = self.options.videos_by_year;
        let help = if detailed {
            tr(
                "The year comes from the name or the video itself. A video with the same name as a photo (Live Photo) stays next to it.",
                "السنة من اسم المقطع أو من بيانات الفيديو نفسه. والفيديو الذي يحمل اسم صورة (صورة حية) يبقى بجانبها.",
            )
        } else {
            tr("Live Photo videos stay next to their photo.", "مقاطع الصور الحية تبقى بجانب صورتها.")
        };
        if theme::toggle_row(ui, true, &mut by_year, tr("Sort loose videos into year folders", "رتّب الفيديو المتفرق في مجلدات السنوات"), help) {
            self.set_option(|o| o.videos_by_year = by_year);
        }
        ui.add_space(8.0);
        let mut rename = self.options.rename_files;
        let help = if detailed {
            tr(
                "Removes codes like [AUD-…] and (1), fixes garbled text, and names files like 4749d951.mp3 from their context.",
                "يحذف الأكواد مثل [AUD-…] و(1)، ويصلح النص المشوّه، ويعطي الملفات مثل 4749d951.mp3 أسماء من سياقها.",
            )
        } else {
            tr("Off: files keep their original names.", "عند إيقافه تبقى الأسماء الأصلية.")
        };
        if theme::toggle_row(ui, true, &mut rename, tr("Clean up file names", "تحسين أسماء الملفات"), help) {
            self.set_option(|o| o.rename_files = rename);
        }
    }

    /// بطاقة نتيجة آخر عملية. ترجع false عند إغلاقها
    fn outcome_view(&mut self, ui: &mut egui::Ui, outcome: &Outcome) -> bool {
        let p = pal();
        let mut keep = true;
        theme::card(ui, |ui| {
            let (icon, color) = if outcome.ok { (Icon::Check, p.success) } else { (Icon::Warning, p.danger) };
            theme::row(ui, |ui| {
                // دائرة خلف الأيقونة بلون النتيجة
                let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 20.0, color.gamma_multiply(0.18));
                icons::paint(ui.painter(), rect.shrink(9.0), icon, color, 0.0);
                let first = outcome.message.lines().next().unwrap_or_default();
                // الرمز في بداية السطر يظهر مرة واحدة في الأيقونة
                let first = first.trim_start_matches(['✔', '↩', '⚠', '❌']).trim_start();
                theme::para(ui, first, 16.0, p.text, true);
            });
            if outcome.folder.exists() {
                ui.label(theme::muted(rtl::for_display(&outcome.folder.display().to_string())).small());
            }
            if let Some(stats) = self.last_stats {
                ui.add_space(8.0);
                theme::row(ui, |ui| {
                    let width = ((ui.available_width() - 3.0 * 30.0 - 2.0 * 10.0) / 3.0).max(90.0);
                    theme::stat(ui, width, Icon::ArrowRight, stats.moved, tr("moved", "نُقل"), p.accent);
                    theme::stat(ui, width, Icon::Edit, stats.renamed, tr("renamed", "أُعيد تسميته"), p.text);
                    theme::stat(ui, width, Icon::Document, stats.duplicates, tr("to the review folder", "إلى المكررات"), p.muted);
                });
                if stats.left > 0 {
                    let note = format!(
                        "{} {}",
                        count(stats.left, Noun::File),
                        tr("stayed: their category is switched off.", "بقيت: فئتها موقوفة.")
                    );
                    ui.label(theme::muted(rtl::for_display(&note)));
                }
            }
            // مكررات «مكررات للمراجعة»: تُعرض للتخلص منها فقط إن كان لكل واحدة توأم خارجها
            if outcome.folder.join(organizer::DUPLICATES_FOLDER).is_dir() {
                if self.review_scan.is_none() {
                    self.review_scan = Some(OrganizerEngine::scan_review(&outcome.folder));
                }
                if let Some(scan) = &self.review_scan {
                    if !scan.orphans.is_empty() {
                        ui.add_space(8.0);
                        let note = format!(
                            "{} — {}",
                            count(scan.orphans.len(), Noun::File),
                            tr("no original outside — left untouched", "لا أصل لها — لم تُلمس")
                        );
                        ui.label(theme::muted(rtl::for_display(&note)));
                    }
                    if !scan.movable.is_empty() {
                        ui.add_space(8.0);
                        let label = if ar() {
                            format!("انقل {} من المكررات ({}) إلى سلة المحذوفات", scan.movable.len(), size_label(scan.size))
                        } else {
                            let n = scan.movable.len();
                            format!("Move {n} duplicate{} ({}) to the Recycle Bin", if n == 1 { "" } else { "s" }, size_label(scan.size))
                        };
                        if theme::button(ui, Kind::Secondary, Some(Icon::Document), &label, true).clicked() {
                            // الفحص لحظة الضغط لا من نتيجة سابقة
                            self.review_scan = Some(OrganizerEngine::scan_review(&outcome.folder));
                            self.trash_folder = Some(outcome.folder.clone());
                            self.confirm_trash = true;
                        }
                    }
                }
            }
            for line in outcome.message.lines().skip(1) {
                let line = line.trim_start_matches(['✔', '↩', '⚠', '❌']).trim_start();
                theme::row(ui, |ui| {
                    icons::show(ui, Icon::Warning, 16.0, p.amber);
                    theme::para(ui, line, 15.0, p.amber, false);
                });
            }
            ui.add_space(10.0);
            theme::row(ui, |ui| {
                if outcome.folder.exists() && theme::button(ui, Kind::Secondary, Some(Icon::External), tr("Open folder", "فتح المجلد"), true).clicked() {
                    self.open_in_explorer(&outcome.folder.clone());
                }
                if let Some(n) = self.undo_count {
                    let label = format!("{} ({})", tr("Undo", "تراجع"), count(n, Noun::File));
                    if theme::button(ui, Kind::Secondary, Some(Icon::Undo), &label, true).clicked() {
                        self.confirm_undo = true;
                    }
                }
                if theme::button(ui, Kind::Ghost, Some(Icon::Folder), tr("Organize another folder", "رتّب مجلداً آخر"), true).clicked() {
                    self.selected_path = None;
                    keep = false;
                }
                if theme::button(ui, Kind::Ghost, Some(Icon::Close), tr("Close", "إغلاق"), true).clicked() {
                    keep = false;
                }
            });
        });
        keep
    }

    fn progress_view(&self, ui: &mut egui::Ui) {
        let p = pal();
        theme::card(ui, |ui| match &self.job {
            Some(Job::Planning(..)) => {
                theme::row(ui, |ui| {
                    icons::spinner(ui, 22.0, p.accent);
                    ui.label(theme::strong(t("Scanning files…", "جارٍ فحص الملفات…"), 17.0));
                });
            }
            Some(Job::Moving { restoring, progress, total, .. }) => {
                let done = progress.load(Ordering::Relaxed);
                let fraction = if *total == 0 { 1.0 } else { done as f32 / *total as f32 };
                let label = if *restoring { t("Restoring…", "جارٍ الإرجاع…") } else { t("Organizing…", "جارٍ الترتيب…") };
                theme::row(ui, |ui| {
                    icons::spinner(ui, 22.0, p.accent);
                    ui.label(theme::strong(label, 17.0));
                });
                ui.add_space(8.0);
                ui.add(egui::ProgressBar::new(fraction).text(format!("{} / {}", done, total)).fill(p.primary));
            }
            None => {}
        });
    }

    fn preview_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(preview) = &self.preview else { return };
        let p = pal();
        let mut decision: Option<bool> = None;
        let mut toggle_skipped = false;
        let plan = &preview.plan;

        Self::page_header(
            ui,
            tr("Review the changes", "راجع التغييرات"),
            tr(
                "Nothing has moved yet. Check the list, then organize — you can undo it later.",
                "لم يتحرك أي ملف بعد. راجع القائمة ثم رتّب — ويمكنك التراجع لاحقاً.",
            ),
        );

        // الأرقام الأساسية
        theme::row(ui, |ui| {
            // عرض كل مربع = (المتاح - الهوامش والحدود 3×30 - المسافات 2×10) ÷ 3
            let width = ((ui.available_width() - 3.0 * 30.0 - 2.0 * 10.0) / 3.0).max(100.0);
            theme::stat(ui, width, Icon::ArrowRight, plan.moves.len(), tr("to move or rename", "للنقل أو إعادة التسمية"), p.accent);
            theme::stat(ui, width, Icon::Check, plan.kept_in_folders + plan.in_place, tr("stay where they are", "تبقى في مكانها"), p.text);
            theme::stat(ui, width, Icon::Shield, plan.skipped.len(), tr("protected folders", "مجلدات محمية"), p.success);
        });
        ui.add_space(12.0);

        // نسخ مكررة: لا حذف، تذهب إلى «مكررات للمراجعة»
        if plan.duplicates > 0 {
            let note = if ar() {
                format!(
                    "{} إلى «مكررات للمراجعة» — لا حذف، ويمكن التراجع.",
                    count(plan.duplicates, Noun::File)
                )
            } else {
                format!(
                    "{} go to the review folder — nothing is deleted, and you can undo.",
                    count(plan.duplicates, Noun::File)
                )
            };
            ui.label(theme::muted(rtl::for_display(&note)));
            ui.add_space(6.0);
        }

        // مجلد سحابي: النقل فيه يعني رفعاً وتنزيلاً
        if let Some(provider) = SafetyChecker::cloud_provider(&plan.source) {
            let warning = if ar() {
                format!(
                    "⚠ هذا المجلد داخل {provider}: النقل فيه يعني رفعاً وتنزيلاً. العمليات المتوقعة: {}.",
                    count(plan.moves.len(), Noun::File)
                )
            } else {
                format!(
                    "⚠ This folder is inside {provider}: moving files there means uploading and downloading. Expected operations: {}.",
                    count(plan.moves.len(), Noun::File)
                )
            };
            ui.label(theme::muted(rtl::for_display(&warning)));
            ui.add_space(6.0);
        }

        // إلى أين تذهب الملفات
        let wrap_layout = if ar() {
            egui::Layout::right_to_left(egui::Align::Min).with_main_wrap(true)
        } else {
            egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true)
        };
        ui.with_layout(wrap_layout, |ui| {
            for (folder, n, existing, artists) in &preview.destinations {
                let name = if folder == THIS_FOLDER { tr("This folder", "هذا المجلد").to_string() } else { folder.clone() };
                // الاسم معزول حتى لا يلتصق به الرقم بعده فيُقرأ اسم إنجليزي مع أرقامه بالمقلوب في العربية
                let mut label = format!("\u{2068}{name}\u{2069}  ·  {n}");
                if *artists > 0 {
                    label.push_str(&format!("  ·  {} {}", count(*artists, Noun::Folder), tr("inside", "بالداخل")));
                } else if !existing {
                    label.push_str(&format!("  ·  {}", tr("new", "جديد")));
                }
                let shown = if ar() { rtl::rtl_line(&label) } else { rtl::for_display(&label.replace(['\u{2068}', '\u{2069}'], "")) };
                theme::chip(ui, category_icon(folder), &shown);
            }
        });
        ui.add_space(4.0);
        if !preview.skipped.is_empty() {
            let label = if ar() {
                format!("{} لن يُلمس", count(preview.skipped.len(), Noun::ProtectedFolder))
            } else {
                format!("{} will not be touched", count(preview.skipped.len(), Noun::ProtectedFolder))
            };
            let response = theme::row(ui, |ui| {
                icons::show(ui, Icon::Shield, 16.0, p.success);
                ui.add(egui::Label::new(RichText::new(theme::show(&label)).color(p.success)).sense(egui::Sense::click()))
            })
            .inner;
            if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                toggle_skipped = true;
            }
            if self.show_skipped {
                egui::ScrollArea::vertical().id_source("skipped_list").max_height(90.0).show(ui, |ui| {
                    theme::column(ui, |ui| {
                        for line in &preview.skipped {
                            ui.label(RichText::new(rtl::for_display(line)).small());
                        }
                    });
                });
            }
        }
        ui.add_space(10.0);

        // ملخص: بطاقة لكل فئة ولكل مجموعة تسمية وللمكررات
        if !preview.plan.left.is_empty() {
            let note = format!(
                "{} {}",
                count(preview.plan.left.len(), Noun::File),
                tr("stay where they are: their category is switched off.", "تبقى في مكانها: فئتها موقوفة.")
            );
            ui.label(theme::muted(rtl::for_display(&note)));
            ui.add_space(6.0);
        }
        let (show_all, toggled) = if self.show_all { (false, None) } else { summary_cards(ui, &preview.plan, &self.options) };
        if show_all {
            self.show_all = true;
        }
        ui.add_space(8.0);

        if self.show_all {
        // الجدول
        let row_height = ui.text_style_height(&egui::TextStyle::Body) + 12.0;
        let table_height = (ui.available_height() - 100.0).max(row_height * 4.0);
        theme::card(ui, |ui| {
            let column = ((ui.available_width() - 100.0) / 2.0).max(60.0);
            theme::row(ui, |ui| {
                table_cell(ui, column, 20.0, &t("CURRENT LOCATION", "المكان الحالي"), p.muted, true);
                ui.add_space(20.0);
                table_cell(ui, column, 20.0, &t("NEW LOCATION", "المكان الجديد"), p.muted, true);
            });
            ui.add(egui::Separator::default().spacing(10.0));
            egui::ScrollArea::vertical()
                .max_height(table_height - 56.0)
                .auto_shrink([false, false])
                .show_rows(ui, row_height, preview.rows.len(), |ui, range| {
                    let start = range.start;
                    for (i, (from, to)) in preview.rows[range].iter().enumerate() {
                        let rect = ui.available_rect_before_wrap();
                        if (start + i) % 2 == 1 {
                            ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), row_height)), 6.0, p.stripe);
                        }
                        theme::row(ui, |ui| {
                            table_cell(ui, column, row_height, from, p.muted, false);
                            icons::show(ui, Icon::ArrowRight, 16.0, p.accent);
                            table_cell(ui, column, row_height, &to, p.text, false);
                        });
                    }
                });
        });
        ui.add_space(12.0);

        }
        theme::row(ui, |ui| {
            let label = format!("{} {}", tr("Organize", "رتّب"), count(preview.rows.len(), Noun::File));
            if theme::button(ui, Kind::Primary, Some(Icon::Check), &label, true).clicked() {
                decision = Some(true);
            }
            if theme::button(ui, Kind::Secondary, Some(Icon::Close), tr("Cancel", "إلغاء"), true).clicked() {
                decision = Some(false);
            }
            let view_label = if self.show_all { tr("Summary", "الملخص") } else { tr("Show all", "عرض الكل") };
            if theme::button(ui, Kind::Ghost, Some(Icon::History), view_label, true).clicked() {
                self.show_all = !self.show_all;
            }
        });

        if toggle_skipped {
            self.show_skipped = !self.show_skipped;
        }
        // مفتاح تغيّر من بطاقة: يُحفظ ويُعاد حساب الخطة قبل أي تنفيذ
        if let Some((kind, on)) = toggled {
            match kind {
                CardKind::Category(key) => self.options.set_category(key, on),
                CardKind::Naming(group) => self.options.set_naming_group(group, on),
                CardKind::Duplicates => self.options.move_duplicates = on,
            }
            if let Err(err) = self.storage.save_settings(&self.options) {
                self.status_message = format!("⚠ {err}");
            }
            if let Some(path) = self.selected_path.clone() {
                self.preview = None;
                self.start_planning(ctx, path);
            }
            return;
        }
        match decision {
            Some(true) => {
                if let Some(preview) = self.preview.take() {
                    self.start_organizing(ctx, preview.plan);
                }
            }
            Some(false) => {
                self.preview = None;
                self.status_message = tr("Cancelled. No files were moved.", "أُلغي. لم يُنقل أي ملف.").to_string();
            }
            None => {}
        }
    }

    fn history_page(&mut self, ui: &mut egui::Ui) {
        let p = pal();
        Self::page_header(ui, tr("History", "السجل"), tr("Undo the last organize and put every file back where it was.", "تراجع عن آخر ترتيب وأعِد كل ملف إلى مكانه."));
        let last = self.undo_source.clone().zip(self.undo_count);
        theme::card(ui, |ui| match last {
            Some((source, n)) => {
                theme::section_title(ui, Icon::History, &t("Last organize", "آخر ترتيب"));
                ui.add_space(8.0);
                theme::icon_text(ui, Icon::Folder, theme::strong(rtl::for_display(&folder_name(&source)), 15.0), p.muted);
                ui.label(theme::muted(rtl::for_display(&source.display().to_string())).small());
                if let Some(when) = self.storage.undo_saved_at() {
                    let local: DateTime<Local> = when.into();
                    theme::icon_text(ui, Icon::Clock, theme::muted(local.format("%Y-%m-%d  %H:%M").to_string()), p.muted);
                }
                let back = if ar() { format!("يمكن إرجاع {}", count(n, Noun::File)) } else { format!("{} can be moved back", count(n, Noun::File)) };
                theme::icon_text(ui, Icon::Undo, theme::muted(theme::show(&back)), p.muted);
                ui.add_space(12.0);
                theme::row(ui, |ui| {
                    if theme::button(ui, Kind::Primary, Some(Icon::Undo), tr("Undo last organize", "تراجع عن آخر ترتيب"), true).clicked() {
                        self.confirm_undo = true;
                    }
                    if theme::button(ui, Kind::Secondary, Some(Icon::External), tr("Open folder", "فتح المجلد"), true).clicked() {
                        self.open_in_explorer(&source);
                    }
                });
                ui.add_space(6.0);
                theme::para(ui, tr("Files you changed or replaced since then are left alone.", "الملفات التي غيّرتها أو استبدلتها بعد ذلك تبقى كما هي."), 13.0, p.muted, false);
            }
            None => {
                ui.vertical_centered(|ui| {
                    ui.add_space(10.0);
                    icons::show(ui, Icon::History, 40.0, p.muted);
                    ui.label(theme::strong(t("Nothing to undo", "لا يوجد ما يُتراجع عنه"), 17.0));
                    ui.label(theme::muted(t(
                        "After you organize a folder, you can put everything back from here.",
                        "بعد ترتيب أي مجلد، يمكنك إعادة كل شيء من هنا.",
                    )));
                    ui.add_space(10.0);
                });
            }
        });
    }

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        Self::page_header(ui, tr("Settings", "الإعدادات"), tr("Saved automatically and used from the next preview.", "تُحفظ تلقائياً وتُستخدم من المعاينة التالية."));
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            theme::column(ui, |ui| {
                theme::card(ui, |ui| {
                    theme::section_title(ui, Icon::About, &t("Language", "اللغة"));
                    ui.add_space(8.0);
                    let mut selected = if ar() { 0 } else { 1 };
                    if theme::segmented(ui, &["العربية", "English"], &mut selected) {
                        self.set_language(selected == 0);
                    }
                });
                ui.add_space(14.0);

                theme::card(ui, |ui| {
                    theme::section_title(ui, Icon::Settings, &t("Appearance", "المظهر"));
                    ui.add_space(8.0);
                    let mut selected = match self.options.appearance.as_str() {
                        "light" => 1,
                        "dark" => 2,
                        _ => 0,
                    };
                    let names = [tr("System", "حسب النظام"), tr("Light", "فاتح"), tr("Dark", "داكن")];
                    if theme::segmented(ui, &names, &mut selected) {
                        let value = ["system", "light", "dark"][selected].to_string();
                        self.set_option(move |o| o.appearance = value.clone());
                    }
                });
                ui.add_space(14.0);

                theme::card(ui, |ui| {
                    theme::section_title(ui, Icon::Organize, &t("Organizing", "الترتيب"));
                    ui.add_space(8.0);
                    self.option_toggles(ui, false);
                });
                ui.add_space(14.0);

                theme::card(ui, |ui| {
                    theme::section_title(ui, Icon::Music, &t("Music grouping", "تجميع الموسيقى"));
                    theme::para(
                        ui,
                        tr("Words that mark a speaker in a file name, and names of AI music tools.", "كلمات تدل على صاحب المقطع في اسم الملف، وأسماء أدوات صنع الموسيقى بالذكاء الاصطناعي."),
                        15.0,
                        pal().muted,
                        false,
                    );
                    ui.add_space(8.0);
                    let speakers = format!("{}  {}", tr("Speaker words:", "كلمات أصحاب المقاطع:"), self.options.speaker_keywords.join("، "));
                    let ai_tools = format!("{}  {}", tr("AI tools:", "أدوات الذكاء الاصطناعي:"), self.options.ai_markers.join(", "));
                    ui.label(RichText::new(theme::show(&speakers)).small());
                    ui.label(RichText::new(theme::show(&ai_tools)).small());
                    ui.add_space(8.0);
                    if theme::button(ui, Kind::Secondary, Some(Icon::Edit), tr("Edit lists in Notepad", "تعديل القوائم في Notepad"), true).clicked() {
                        self.open_settings_file();
                    }
                });
                ui.add_space(14.0);

                theme::card(ui, |ui| {
                    theme::section_title(ui, Icon::Lock, &t("Data on this PC", "البيانات على جهازك"));
                    theme::para(
                        ui,
                        tr("Settings and the undo record are kept in:", "الإعدادات وسجل التراجع محفوظة في:"),
                        13.0,
                        pal().muted,
                        false,
                    );
                    ui.label(theme::muted(self.storage.dir().display().to_string()).small());
                    ui.add_space(8.0);
                    if theme::button(ui, Kind::Secondary, Some(Icon::External), tr("Open data folder", "فتح مجلد البيانات"), true).clicked() {
                        let dir = self.storage.dir().to_path_buf();
                        let _ = std::fs::create_dir_all(&dir);
                        self.open_in_explorer(&dir);
                    }
                });
            });
        });
    }

    fn about_page(&mut self, ui: &mut egui::Ui) {
        let p = pal();
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            if let Some(logo) = &self.logo {
                ui.add(egui::Image::from_texture(egui::load::SizedTexture::new(logo.id(), [112.0, 112.0])));
            }
            ui.add_space(6.0);
            ui.label(theme::strong("Array", 32.0));
            ui.label(theme::muted(t(&format!("Version {}", VERSION), &format!("الإصدار {}", VERSION))));
            ui.add_space(14.0);
            for (en, arabic) in [
                ("Gathers scattered files and puts them in tidy folders —", "يجمع الملفات المبعثرة ويضعها في مجلدات مرتبة —"),
                ("recitations and songs into each artist's folder —", "التلاوات والأغاني في مجلد كل قارئ وفنان —"),
                ("without touching what you already organized.", "دون أن يمس ما رتّبته أنت."),
            ] {
                ui.label(t(en, arabic));
            }
            ui.add_space(18.0);
            // سطر بأيقونة في المنتصف: عرضه يُحسب ثم تُترك مسافة مساوية على الجانبين
            let note = t("Everything runs on this computer. Nothing is uploaded.", "كل شيء يعمل على هذا الجهاز. لا يُرفع أي شيء.");
            let width = ui.painter().layout_no_wrap(note.clone(), egui::FontId::proportional(15.0), p.success).size().x + 26.0;
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                if ar() {
                    ui.label(RichText::new(note.clone()).color(p.success));
                    icons::show(ui, Icon::Lock, 16.0, p.success);
                } else {
                    icons::show(ui, Icon::Lock, 16.0, p.success);
                    ui.label(RichText::new(note.clone()).color(p.success));
                }
            });
            ui.add_space(14.0);
            if theme::button(ui, Kind::Ghost, Some(Icon::About), tr("Show the welcome screen", "عرض شاشة الترحيب"), true).clicked() {
                self.set_option(|o| o.welcome_seen = false);
            }
            ui.add_space(14.0);
        });
    }

    /// شاشة الترحيب في أول تشغيل: اختيار اللغة والخطوات الثلاث
    fn welcome_screen(&mut self, ui: &mut egui::Ui) {
        let p = pal();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(24.0);
                if let Some(logo) = &self.logo {
                    ui.add(egui::Image::from_texture(egui::load::SizedTexture::new(logo.id(), [96.0, 96.0])));
                }
                ui.add_space(8.0);
                ui.label(theme::strong(t("Welcome to Array", "أهلاً بك في Array"), 30.0));
                ui.label(theme::muted(t(
                    "Tidy up scattered files in three steps — safely, on this PC.",
                    "رتّب ملفاتك المبعثرة في ثلاث خطوات — بأمان وعلى جهازك.",
                )));
                ui.add_space(14.0);
                let mut selected = if ar() { 0 } else { 1 };
                if theme::segmented(ui, &["العربية", "English"], &mut selected) {
                    self.set_language(selected == 0);
                }
                ui.add_space(22.0);
            });

            // البطاقات الثلاث متجاورة، والأولى عند بداية السطر
            let steps = [
                (Icon::Folder, "1", tr("Choose a folder", "اختر مجلداً"), tr("Pick it with Browse or drag it onto the window.", "اختره بزر الاستعراض أو اسحبه إلى النافذة.")),
                (
                    Icon::Search,
                    "2",
                    tr("Review the preview", "راجع المعاينة"),
                    tr("See where every file will go. Nothing moves yet.", "ترى أين سيذهب كل ملف. لا يتحرك شيء بعد."),
                ),
                (
                    Icon::Check,
                    "3",
                    tr("Organize — and undo anytime", "رتّب — وتراجع متى شئت"),
                    tr("Confirm to organize. The History page puts everything back.", "أكّد للترتيب. وصفحة السجل تعيد كل شيء كما كان."),
                ),
            ];
            let gap = 16.0;
            let card_width = ((ui.available_width().min(980.0) - 2.0 * gap) / 3.0 - 42.0).max(180.0);
            let total = 3.0 * (card_width + 42.0) + 2.0 * gap;
            theme::row(ui, |ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                for (icon, number, title, body) in steps {
                    egui::Frame::none()
                        .fill(p.surface)
                        .stroke(egui::Stroke::new(1.0_f32, p.border))
                        .rounding(14.0)
                        .inner_margin(egui::Margin::same(20.0))
                        .show(ui, |ui| {
                            ui.set_width(card_width);
                            ui.set_min_height(150.0);
                            theme::column(ui, |ui| {
                                theme::row(ui, |ui| {
                                    let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
                                    ui.painter().circle_filled(rect.center(), 20.0, p.selected);
                                    icons::paint(ui.painter(), rect.shrink(10.0), icon, p.accent, 0.0);
                                    ui.label(theme::strong(number, 22.0).color(p.accent));
                                });
                                ui.add_space(6.0);
                                ui.label(theme::strong(theme::show(title), 16.0));
                                theme::para(ui, body, 14.0, p.muted, false);
                            });
                        });
                }
            });

            ui.add_space(22.0);
            ui.vertical_centered(|ui| {
                let note = t(
                    "Your existing folders stay as they are, and everything runs offline.",
                    "مجلداتك الحالية تبقى كما هي، وكل شيء يعمل دون إنترنت.",
                );
                ui.label(theme::muted(note));
                ui.add_space(14.0);
                if theme::button(ui, Kind::Primary, Some(Icon::ArrowRight), tr("Get started", "ابدأ"), true).clicked() {
                    self.set_option(|o| o.welcome_seen = true);
                    self.page = Page::Organize;
                }
                ui.add_space(20.0);
            });
        });
    }

    fn dialog(&mut self, ctx: &egui::Context, id: &str, title: &str, body: &str, note: &str) -> Option<bool> {
        let p = pal();
        let mut answer = None;
        egui::Window::new(id)
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .fixed_size([440.0, 0.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                theme::column(ui, |ui| {
                    ui.label(theme::strong(theme::show(title), 18.0));
                    ui.add_space(6.0);
                    theme::para(ui, body, 15.0, p.text, false);
                    theme::para(ui, note, 13.0, p.muted, false);
                    ui.add_space(12.0);
                    theme::row(ui, |ui| {
                        if theme::button(ui, Kind::Primary, None, title, true).clicked() {
                            answer = Some(true);
                        }
                        if theme::button(ui, Kind::Secondary, None, tr("Cancel", "إلغاء"), true).clicked() {
                            answer = Some(false);
                        }
                    });
                });
            });
        answer
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.confirm_undo {
            let n = count(self.undo_count.unwrap_or(0), Noun::File);
            let body = if ar() {
                format!("هل تريد إعادة {} إلى أماكنها قبل آخر ترتيب؟", n)
            } else {
                format!("Move {} back to where they were before the last organize?", n)
            };
            let note = tr("Files that were changed or replaced since then are left alone.", "الملفات التي تغيّرت أو استُبدلت بعد ذلك تبقى كما هي.");
            match self.dialog(ctx, "undo", tr("Undo last organize", "تراجع عن آخر ترتيب"), &body, note) {
                Some(true) => {
                    self.confirm_undo = false;
                    self.start_undo(ctx);
                }
                Some(false) => self.confirm_undo = false,
                None => {}
            }
        }
        if self.confirm_trash {
            let (n, size) = self.review_scan.as_ref().map(|s| (s.movable.len(), s.size)).unwrap_or((0, 0));
            let body = if ar() {
                format!("هل تريد نقل {n} من المكررات ({}) إلى سلة المحذوفات؟", size_label(size))
            } else {
                format!("Move {n} duplicate{} ({}) to the Recycle Bin?", if n == 1 { "" } else { "s" }, size_label(size))
            };
            let note = tr(
                "You can restore them from the Recycle Bin. Files with no original outside the review folder are not touched.",
                "يمكنك استرجاعها من سلة المحذوفات. الملفات التي لا أصل لها خارج مجلد المراجعة لا تُلمس.",
            );
            match self.dialog(ctx, "trash", tr("Move duplicates to the Recycle Bin", "نقل المكررات إلى سلة المحذوفات"), &body, note) {
                Some(true) => {
                    self.confirm_trash = false;
                    self.trash_review_duplicates();
                }
                Some(false) => self.confirm_trash = false,
                None => {}
            }
        }
    }

    /// نقل مكررات المراجعة إلى السلة: يُعاد الفحص لحظة التنفيذ، ولا يُنقل إلا ما له توأم
    fn trash_review_duplicates(&mut self) {
        let Some(folder) = self.trash_folder.clone() else { return };
        let scan = OrganizerEngine::scan_review(&folder);
        let available = scan.movable.len();
        if available == 0 {
            self.review_scan = None;
            self.status_message = tr("Nothing to move: no duplicate has an original outside the review folder.", "لا شيء يُنقل: لا توأم لأي مكرر خارج مجلد المراجعة.").to_string();
            return;
        }
        self.review_scan = None;
        let (moved, error) = OrganizerEngine::trash_files(&scan.movable);
        self.status_message = match (moved, error) {
            (n, None) => {
                if ar() {
                    format!("نُقل {n} إلى سلة المحذوفات — يمكنك استرجاعها من السلة.")
                } else {
                    format!("{n} moved to the Recycle Bin — you can restore them from there.")
                }
            }
            (0, Some(err)) => format!("⚠ {err}"),
            (n, Some(err)) => {
                if ar() {
                    format!("نُقل {n} إلى السلة، وفشل الباقي: {err}")
                } else {
                    format!("Moved {n} to the Recycle Bin; the rest failed: {err}")
                }
            }
        };
    }
}

/// حجم مقروء بالأرقام الغربية دائماً: "12.3 MB"، ومعزول الاتجاه حتى لا يقلبه
/// النص العربي إلى "MB 12.3"
fn size_label(bytes: u64) -> String {
    let text = if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    };
    format!("\u{2066}{text}\u{2069}")
}

/// أيقونة مجلد الوجهة في المعاينة حسب نوعه
/// مفتاح بطاقة في الملخص
#[derive(Clone, Copy)]
enum CardKind {
    Category(&'static str),
    Naming(NamingGroup),
    Duplicates,
}

struct CardInput {
    icon: Icon,
    title: String,
    summary: Summary,
    enabled: bool,
    kind: CardKind,
}

/// بطاقات الملخص: العدد، وخمسة أمثلة «قبل ← بعد»، ومفتاح، وزر عرض الكل.
/// تُرجع (هل ضُغط «عرض الكل»؟، وأي مفتاح تغيّر إلى أي حالة)
fn summary_cards(ui: &mut egui::Ui, plan: &Plan, options: &Options) -> (bool, Option<(CardKind, bool)>) {
    let p = pal();
    let mut cards: Vec<CardInput> = Vec::new();
    for (key, summary) in &plan.categories {
        cards.push(CardInput {
            icon: category_key_icon(key),
            title: category_title(key),
            summary: summary.clone(),
            enabled: options.category_enabled(key),
            kind: CardKind::Category(key),
        });
    }
    for (group, summary) in &plan.naming_groups {
        let (icon, title) = naming_group_card(*group);
        cards.push(CardInput {
            icon,
            title,
            summary: summary.clone(),
            enabled: options.naming_enabled(*group),
            kind: CardKind::Naming(*group),
        });
    }
    if plan.duplicates > 0 {
        let review = plan.source.join(organizer::DUPLICATES_FOLDER);
        let examples = plan
            .moves
            .iter()
            .filter(|m| m.to.starts_with(&review))
            .take(5)
            .map(|m| {
                (
                    m.from.file_name().unwrap_or_default().to_string_lossy().to_string(),
                    m.to.strip_prefix(&plan.source).unwrap_or(&m.to).display().to_string(),
                )
            })
            .collect();
        cards.push(CardInput {
            icon: Icon::Document,
            title: tr("Duplicates", "المكررات").to_string(),
            summary: Summary { count: plan.duplicates, examples },
            enabled: options.move_duplicates,
            kind: CardKind::Duplicates,
        });
    }

    let mut show_all = false;
    let mut toggled: Option<(CardKind, bool)> = None;
    let width = ((ui.available_width() - 70.0) / 2.0).max(230.0);
    {
        {
            for pair in cards.chunks(2) {
                theme::row(ui, |ui| {
                    for card in pair {
                        ui.allocate_ui_with_layout(
                            egui::vec2(width, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                theme::card(ui, |ui| {
                                    ui.set_width((width - 30.0).max(180.0));
                                    theme::row(ui, |ui| {
                                        icons::show(ui, card.icon, 18.0, p.accent);
                                        ui.label(theme::strong(rtl::for_display(&card.title), 15.0));
                                        ui.label(theme::strong(card.summary.count.to_string(), 15.0).color(p.accent));
                                    });
                                    ui.add_space(6.0);
                                    for (old, new) in &card.summary.examples {
                                        let line = format!("{old}  ←  {new}");
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(rtl::for_display(&line)).small().color(p.muted),
                                            )
                                            .truncate(true),
                                        );
                                    }
                                    ui.add_space(8.0);
                                    let mut on = card.enabled;
                                    let state = if on { tr("Included", "مشمول") } else { tr("Skipped", "مستثنى") };
                                    if theme::toggle_row(ui, true, &mut on, state, "") {
                                        toggled = Some((card.kind, on));
                                    }
                                    ui.add_space(4.0);
                                    if theme::button(ui, Kind::Ghost, Some(Icon::ArrowRight), tr("Show all", "عرض الكل"), true).clicked() {
                                        show_all = true;
                                    }
                                });
                            },
                        );
                        ui.add_space(18.0);
                    }
                });
                ui.add_space(10.0);
            }
        }
    }
    (show_all, toggled)
}

fn category_key_icon(key: &str) -> Icon {
    match key {
        "Pictures" => Icon::Photo,
        "Videos" => Icon::Video,
        "Music" => Icon::Music,
        "Documents" => Icon::Document,
        "Archives" => Icon::Archive,
        "Installers" => Icon::Organize,
        "Recordings" => Icon::Clock,
        _ => Icon::Folder,
    }
}

fn category_title(key: &str) -> String {
    match key {
        "Pictures" => tr("Pictures", "الصور").to_string(),
        "Videos" => tr("Videos", "الفيديو").to_string(),
        "Music" => tr("Audio", "الصوتيات").to_string(),
        "Documents" => tr("Documents", "المستندات").to_string(),
        "Archives" => tr("Archives", "المضغوطات").to_string(),
        "Installers" => tr("Installers", "المثبّتات").to_string(),
        "Recordings" => tr("Recordings", "التسجيلات").to_string(),
        _ => tr("Other", "أخرى").to_string(),
    }
}

fn naming_group_card(group: NamingGroup) -> (Icon, String) {
    match group {
        NamingGroup::Junk => (Icon::Edit, tr("Vague names", "أسماء رديئة").to_string()),
        NamingGroup::Whatsapp => (Icon::Photo, tr("WhatsApp media", "وسائط واتساب").to_string()),
        NamingGroup::Screenshot => (Icon::Photo, tr("Screenshots", "لقطات الشاشة").to_string()),
        NamingGroup::Camera => (Icon::Photo, tr("Camera names", "أسماء الكاميرا").to_string()),
        NamingGroup::Recording => (Icon::Music, tr("Recordings", "تسجيلات صوتية").to_string()),
        NamingGroup::Document => (Icon::Document, tr("Document titles", "عناوين المستندات").to_string()),
    }
}

fn category_icon(folder: &str) -> Icon {
    let lower = normalizer::normalize_arabic(&folder.to_lowercase());
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    if has(&["music", "موسيق", "اغاني", "صوتيات", "منوعات", "ذكاء"]) {
        Icon::Music
    } else if has(&["picture", "photo", "image", "صور"]) {
        Icon::Photo
    } else if has(&["video", "movie", "فيديو", "افلام"]) {
        Icon::Video
    } else if has(&["document", "docs", "مستندات", "وثائق"]) {
        Icon::Document
    } else if has(&["archive", "ارشيف", "مضغوط"]) {
        Icon::Archive
    } else {
        Icon::Folder
    }
}

/// اسم المجلد وحده (المسار الكامل يظهر بخط أصغر تحته)
fn folder_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string())
}

/// خلية بعرض ثابت. النص بترتيبه المنطقي؛ إن كان أطول من الخلية يُقص من نهايته المنطقية
/// ثم يُجهّز للعرض، فتظهر "…" في الجهة الصحيحة لكل لغة. ready = النص جاهز للرسم أصلاً
fn table_cell(ui: &mut egui::Ui, width: f32, height: f32, text: &str, color: egui::Color32, ready: bool) {
    let font = egui::FontId::proportional(if ready { 13.0 } else { 15.0 });
    let display = |s: &str| if ready { s.to_string() } else { rtl::for_display(s) };
    let measure = |s: &str| ui.fonts(|f| f.layout_no_wrap(s.to_string(), font.clone(), color).size().x);
    let mut shown = display(text);
    let mut truncated = false;
    if !ready && measure(&shown) > width {
        let chars: Vec<char> = text.chars().collect();
        let (mut lo, mut hi) = (0usize, chars.len());
        while lo < hi {
            let mid = (lo + hi + 1) / 2;
            let candidate: String = chars[..mid].iter().collect::<String>() + "…";
            if measure(&display(&candidate)) <= width {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        shown = display(&(chars[..lo].iter().collect::<String>() + "…"));
        truncated = true;
    }
    let layout = if ar() { egui::Layout::right_to_left(egui::Align::Center) } else { egui::Layout::left_to_right(egui::Align::Center) };
    ui.allocate_ui_with_layout(egui::vec2(width, height), layout, |ui| {
        ui.set_min_width(width);
        let response = ui.add(egui::Label::new(RichText::new(shown).font(font.clone()).color(color)).wrap(false));
        if truncated {
            response.on_hover_text(display(text));
        }
    });
}

impl eframe::App for ArrayApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // السمة: تتبع ويندوز أو اختيار المستخدم
        let dark = match self.options.appearance.as_str() {
            "light" => false,
            "dark" => true,
            _ => frame.info().system_theme.map_or(true, |t| t == eframe::Theme::Dark),
        };
        if self.dark != Some(dark) {
            theme::apply(ctx, dark);
            self.dark = Some(dark);
        }
        let p = pal();

        self.poll_job();
        if self.job.is_some() {
            // تحديث شريط التقدم أثناء العمل
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // شاشة الترحيب في أول تشغيل
        if !self.options.welcome_seen {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(p.bg).inner_margin(egui::Margin::symmetric(32.0, 24.0)))
                .show(ctx, |ui| self.welcome_screen(ui));
            return;
        }

        // سحب مجلد وإفلاته على النافذة
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if let Some(dir) = dropped.into_iter().find(|p| p.is_dir()) {
            if !self.busy() {
                self.page = Page::Organize;
                self.select_folder(dir);
            }
        }

        self.sidebar(ctx);

        egui::TopBottomPanel::bottom("status")
            .frame(egui::Frame::none().fill(p.sidebar).inner_margin(egui::Margin::symmetric(18.0, 7.0)))
            .show(ctx, |ui| {
                // الرمز في بداية الرسالة يُعرض بأيقونة من مجموعة التطبيق
                let status = self.status_message.lines().next().unwrap_or_default();
                let (icon, color, text) = match status.chars().next() {
                    Some('✔') => (Some(Icon::Check), p.success, &status['✔'.len_utf8()..]),
                    Some('↩') => (Some(Icon::Undo), p.accent, &status['↩'.len_utf8()..]),
                    Some('⚠') => (Some(Icon::Warning), p.amber, &status['⚠'.len_utf8()..]),
                    Some('❌') => (Some(Icon::Warning), p.danger, &status['❌'.len_utf8()..]),
                    Some('⏳') => (Some(Icon::Clock), p.accent, &status['⏳'.len_utf8()..]),
                    _ => (None, p.muted, status),
                };
                theme::row(ui, |ui| {
                    if let Some(icon) = icon {
                        icons::show(ui, icon, 14.0, color);
                    }
                    ui.add(egui::Label::new(theme::muted(theme::show(text.trim_start())).small()).truncate(!ar()));
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(p.bg).inner_margin(egui::Margin::symmetric(32.0, 24.0)))
            .show(ctx, |ui| {
                theme::column(ui, |ui| match self.page {
                    Page::Organize => self.organize_page(ui, ctx),
                    Page::History => self.history_page(ui),
                    Page::Settings => self.settings_page(ui),
                    Page::About => self.about_page(ui),
                });
            });

        self.dialogs(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::organizer::PlannedMove;
    use crate::organizer::tests::temp_tree;
    use std::fs;

    /// ملف نُقل إلى `docs/` + سجل AppData صحيح + بيان مجلد بائت أو تالف
    fn undo_setup(name: &str) -> (PathBuf, Storage) {
        let root = temp_tree(&format!("undo_{name}"), &["a.txt"]);
        let moved = PlannedMove { from: root.join("a.txt"), to: root.join("docs/a.txt") };
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::rename(&moved.from, &moved.to).unwrap();
        let storage = Storage::at(root.join(".appdata"));
        let log = UndoLog { source: root.clone(), moves: vec![moved.clone()], created_dirs: vec![root.join("docs")] };
        storage.save_undo(&log).unwrap();
        if name == "stale" {
            Journal::new(&root, &[moved], &[root.join("docs")]).save().unwrap();
        } else {
            fs::create_dir_all(root.join(".array")).unwrap();
            fs::write(Journal::path(&root), "تالف {").unwrap();
        }
        (root, storage)
    }

    fn assert_undone(root: &Path, storage: &Storage) {
        assert!(root.join("a.txt").is_file(), "الملف لم يُرجع");
        assert!(!root.join("docs/a.txt").exists());
        assert!(storage.load_undo().unwrap().is_none(), "سجل AppData لم يُمسح بعد نجاح التراجع");
    }

    /// اختبار دخاني على مسار المستخدم نفسه: اختيار مجلد → معاينة → تنظيم → تراجع.
    /// يقود دوال الواجهة الحقيقية (planning/execute/undo + poll_job) بلا نافذة.
    #[test]
    fn interface_smoke_test_organize_then_undo() {
        let root = temp_tree("ui_smoke", &["1.pdf", "a/2.pdf", "clip.mp4", "sea.jpg"]);
        let before = tree(&root);
        let ctx = egui::Context::default();
        let mut app = ArrayApp::new(&ctx, Storage::at(temp_tree("ui_smoke_appdata", &[])));

        app.select_folder(root.clone());
        app.start_planning(&ctx, root.clone());
        wait_for_job(&mut app);

        let plan = app.preview.take().expect("لم تُعرض المعاينة").plan;
        let moves = plan.moves.len();
        assert!(moves > 0);
        assert_eq!(app.undo_count, None, "زر التراجع ظاهر قبل التنظيم");

        app.start_organizing(&ctx, plan);
        wait_for_job(&mut app);
        assert_eq!(app.outcome.as_ref().map(|o| o.ok), Some(true), "{}", app.status_message);
        assert_ne!(tree(&root), before, "لم يتحرك أي ملف");
        assert_eq!(app.undo_count, Some(moves), "زر التراجع لم يظهر بعد التنظيم");

        app.start_undo(&ctx);
        wait_for_job(&mut app);
        assert_eq!(app.outcome.as_ref().map(|o| o.ok), Some(true), "{}", app.status_message);
        assert_eq!(tree(&root), before, "التراجع لم يعيد الشجرة كما كانت");
        assert_eq!(app.undo_count, None, "زر التراجع بقي بعد نجاحه");
        fs::remove_dir_all(&root).unwrap();
    }

    /// كل الملفات تحت المجلد (بلا `.array`) لمرتبة — للمقارنة قبل وبعد
    fn tree(root: &Path) -> Vec<String> {
        let mut files: Vec<String> = walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.path().strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"))
            .filter(|f| !f.starts_with(".array/"))
            .collect();
        files.sort();
        files
    }

    /// انتظار انتهاء عمل الخلفية عبر نفس دالة الواجهة
    fn wait_for_job(app: &mut ArrayApp) {
        for _ in 0..300 {
            app.poll_job();
            if app.job.is_none() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("لم ينتهِ العمل في الوقت المتوقع");
    }

    #[test]
    fn undo_targets_the_moved_folder_not_the_stored_path() {
        let root = temp_tree("undo_target", &["1.pdf", "a/2.pdf", "clip.mp4"]);
        let storage = Storage::at(temp_tree("undo_target_appdata", &[]));
        let options = Options { keep_existing_folders: false, ..Options::default() };
        let plan = OrganizerEngine::plan(&root, &options).unwrap();
        let result = OrganizerEngine::execute(&plan, &AtomicUsize::new(0));
        assert_eq!(result.done.len(), plan.moves.len());
        storage
            .save_undo(&UndoLog { source: root.clone(), moves: result.done.clone(), created_dirs: plan.new_dirs.clone() })
            .unwrap();

        // المستخدم نقل مجلد التنظيم بعد الترتيب: السجل والبيان يشيران إلى المكان القديم
        let moved = root.with_file_name(format!("{}_moved", root.file_name().unwrap().to_string_lossy()));
        let _ = fs::remove_dir_all(&moved);
        fs::rename(&root, &moved).unwrap();

        let (entries, movable, source) = undo_state(&storage, Some(&moved)).unwrap();
        assert_eq!(source.as_deref(), Some(moved.as_path()), "التراجع ما زال يشير إلى المسار القديم");
        assert_eq!(entries, movable);
        assert!(movable > 0);

        let log = undo_log_for(&storage, Some(&moved)).unwrap().unwrap();
        assert_eq!(log.source, moved);
        let undone = OrganizerEngine::undo(&log.moves, &log.source, &log.created_dirs, &AtomicUsize::new(0));
        assert!(undone.failed.is_empty(), "{:?}", undone.failed);
        assert_eq!(undone.done.len(), log.moves.len());
        assert!(!root.exists(), "لم يُنشأ المسار القديم من جديد");
        fs::remove_dir_all(&moved).unwrap();
    }

    #[test]
    fn a_damaged_journal_falls_back_to_the_appdata_log() {
        let (root, storage) = undo_setup("damaged");
        let result = undo_folder(&root, &storage, &AtomicUsize::new(0)).unwrap();
        assert_eq!(result.done.len(), 1);
        assert_undone(&root, &storage);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_stale_journal_falls_back_and_keeps_the_appdata_log_until_done() {
        let (root, storage) = undo_setup("stale");
        let result = undo_folder(&root, &storage, &AtomicUsize::new(0)).unwrap();
        assert_eq!(result.done.len(), 1);
        assert_undone(&root, &storage);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn app_icon_decodes() {
        let icon = super::load_app_icon();
        assert!(icon.width >= 32 && icon.height >= 32);
        assert_eq!(icon.rgba.len(), (icon.width * icon.height * 4) as usize);
    }

    #[test]
    fn logo_png_is_valid() {
        let img = image::load_from_memory(include_bytes!("../assets/app-256.png")).unwrap();
        assert_eq!((img.width(), img.height()), (256, 256));
    }
}
