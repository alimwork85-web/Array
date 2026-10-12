#![windows_subsystem = "windows"]

mod faces;
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

use chrono::{DateTime, Local};
use eframe::egui::{self, RichText};

use i18n::{ar, count, tr, Noun};
use icons::Icon;
use organizer::{Options, OrganizerEngine, Plan, PlanProgress};
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
    Planning(Receiver<Result<Plan, String>>, Arc<PlanProgress>),
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
struct Preview {
    plan: Plan,
    /// (اسم المجلد، عدد الملفات، موجود أم جديد، عدد مجلدات الفنانين)
    destinations: Vec<(String, usize, bool, usize)>,
    /// (المكان الحالي، المكان الجديد، درجة الوجه إن وُجدت)
    rows: Vec<(String, String, Option<f32>)>,
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
            .map(|m| (relative(&m.from, &plan.source), relative(&m.to, &plan.source), plan.face_matches.get(&m.from).map(|(_, s)| *s)))
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
    confirm_clear_faces: bool,
    show_skipped: bool,
    /// السمة المطبّقة حالياً (None قبل أول إطار)
    dark: Option<bool>,
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
            confirm_clear_faces: false,
            show_skipped: false,
            dark: None,
        };
        app.refresh_undo();
        app
    }

    fn refresh_undo(&mut self) {
        // بعد تنظيم توقف في المنتصف يحتوي السجل ملفات لم تُنقل، فلا تُحسب
        let (entries, movable, source) = match self.storage.load_undo() {
            Ok(Some(log)) => (log.moves.len(), log.moves.iter().filter(|m| m.to.exists()).count(), Some(log.source)),
            Ok(None) => (0, 0, None),
            Err(err) => {
                self.status_message = format!("⚠ {}", err);
                (0, 0, None)
            }
        };
        self.undo_entries = entries;
        self.undo_count = Some(movable).filter(|&n| n > 0);
        self.undo_source = source;
    }

    fn busy(&self) -> bool {
        self.job.is_some() || self.preview.is_some() || self.confirm_undo
    }

    fn poll_job(&mut self) {
        let Some(job) = &self.job else { return };
        match job {
            Job::Planning(rx, _) => match poll(rx) {
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
        self.outcome = None;
        self.status_message = tr("Ready. Press Preview to see where each file will go.", "جاهز. اضغط معاينة لترى أين سيذهب كل ملف.").to_string();
    }

    fn start_planning(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.reload_settings();
        self.outcome = None;
        let options = self.options.clone();
        let progress = Arc::new(PlanProgress::default());
        let shared = progress.clone();
        let cache = self.storage.face_cache_path();
        self.job = Some(Job::Planning(
            spawn_job(ctx, move || OrganizerEngine::plan_with(&path, &options, &shared, Some(&cache))),
            progress,
        ));
        self.status_message = format!("⏳ {}", tr("Scanning files…", "جارٍ فحص الملفات…"));
    }

    fn start_organizing(&mut self, ctx: &egui::Context, plan: Plan) {
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
        let storage = self.storage.clone();
        let counter = progress.clone();
        let rx = spawn_job(ctx, move || {
            let Some(log) = storage.load_undo()? else {
                return Err(tr("There is nothing to undo.", "لا يوجد ما يمكن التراجع عنه.").to_string());
            };
            let result = OrganizerEngine::undo(&log.moves, &log.source, &log.created_dirs, &counter);
            // الملفات التي لم تُرجع تبقى في السجل لإعادة المحاولة
            if result.failed.is_empty() {
                storage.clear_undo()?;
            } else {
                storage.save_undo(&UndoLog { source: log.source.clone(), moves: result.failed.clone(), created_dirs: log.created_dirs.clone() })?;
            }

            let done = count(result.done.len(), Noun::File);
            let mut msg = if ar() {
                format!("↩ أُعيد {} إلى أماكنها الأصلية في {}", done, folder_name(&log.source))
            } else {
                format!("↩ Restored {} to their original places in {}", done, folder_name(&log.source))
            };
            if !result.failed.is_empty() {
                let failed = count(result.failed.len(), Noun::File);
                msg.push_str(&if ar() {
                    format!("\n⚠ تعذّرت إعادة {} (نُقل أو أُعيدت تسميته أو مفتوح أو يوجد ملف جديد بنفس الاسم). اضغط تراجع مرة أخرى لإعادة المحاولة.", failed)
                } else {
                    format!("\n⚠ {} could not be restored (moved, renamed, in use, or a new file has the same name). Press Undo again to retry them.", failed)
                });
            }
            Ok(msg)
        });
        self.job = Some(Job::Moving { restoring: true, rx, progress, total, folder });
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
        let mut by_faces = self.options.sort_by_faces;
        let help = if detailed {
            tr(
                "Learns faces from folders like Ali or Mom and suggests a photo only when one known person is clearly recognized.",
                "يتعلّم الوجوه من مجلدات مثل Ali أو Mom، ولا يقترح نقل صورة إلا إذا تعرّف بوضوح على شخص معروف واحد.",
            )
        } else {
            tr("Needs 'Keep my existing folders'.", "يحتاج خيار «أبقِ مجلداتي الحالية».")
        };
        if theme::toggle_row(ui, keep, &mut by_faces, tr("Sort loose photos into people folders by face", "رتّب الصور المتفرقة في مجلدات الأشخاص بالوجوه"), help) {
            self.set_option(|o| o.sort_by_faces = by_faces);
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
            Some(Job::Planning(_, progress)) if progress.total.load(Ordering::Relaxed) > 0 => {
                let (done, total) = (progress.done.load(Ordering::Relaxed), progress.total.load(Ordering::Relaxed));
                let stage = progress.stage.lock().unwrap().clone();
                theme::row(ui, |ui| {
                    icons::spinner(ui, 22.0, p.accent);
                    ui.label(theme::strong(theme::show(&stage), 17.0));
                });
                ui.add_space(8.0);
                ui.add(egui::ProgressBar::new(done as f32 / total as f32).text(format!("{} / {}", done, total)).fill(p.primary));
                ui.add_space(4.0);
                theme::row(ui, |ui| {
                    icons::show(ui, Icon::Person, 16.0, p.muted);
                    theme::para(
                        ui,
                        tr("The first time takes a few minutes; faces are remembered for next time.", "المرة الأولى تستغرق دقائق، ثم تُحفظ الوجوه للمرات القادمة."),
                        13.0,
                        p.muted,
                        false,
                    );
                });
            }
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
            // عرض كل مربع = (المتاح - الهوامش والحدود 4×30 - المسافات 3×10) ÷ 4
            let width = ((ui.available_width() - 4.0 * 30.0 - 3.0 * 10.0) / 4.0).max(100.0);
            theme::stat(ui, width, Icon::ArrowRight, plan.moves.len(), tr("to move or rename", "للنقل أو إعادة التسمية"), p.accent);
            theme::stat(ui, width, Icon::Check, plan.kept_in_folders + plan.in_place, tr("stay where they are", "تبقى في مكانها"), p.text);
            theme::stat(ui, width, Icon::Person, plan.face_matches.len(), tr("matched by face", "عُرفت بالوجه"), p.amber);
            theme::stat(ui, width, Icon::Shield, plan.skipped.len(), tr("protected folders", "مجلدات محمية"), p.success);
        });
        ui.add_space(12.0);

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
        if !plan.people.is_empty() {
            let line = format!("{} {}", tr("People learned from your folders:", "الأشخاص الذين تعلّم البرنامج وجوههم:"), plan.people.join(tr(", ", "، ")));
            theme::icon_text(ui, Icon::Person, theme::muted(theme::show(&line)), p.muted);
        }
        if let Some(note) = &plan.face_note {
            theme::icon_text(ui, Icon::About, theme::muted(theme::show(note)).small(), p.muted);
        }
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
                    for (i, (from, to, face)) in preview.rows[range].iter().enumerate() {
                        let rect = ui.available_rect_before_wrap();
                        if (start + i) % 2 == 1 {
                            ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), row_height)), 6.0, p.stripe);
                        }
                        let to = match face {
                            Some(score) => format!("{}   ·  {} {:.0}%", to, tr("face", "وجه"), score * 100.0),
                            None => to.clone(),
                        };
                        theme::row(ui, |ui| {
                            table_cell(ui, column, row_height, from, p.muted, false);
                            icons::show(ui, Icon::ArrowRight, 16.0, p.accent);
                            table_cell(ui, column, row_height, &to, p.text, false);
                        });
                    }
                });
        });
        ui.add_space(12.0);

        theme::row(ui, |ui| {
            let label = format!("{} {}", tr("Organize", "رتّب"), count(preview.rows.len(), Noun::File));
            if theme::button(ui, Kind::Primary, Some(Icon::Check), &label, true).clicked() {
                decision = Some(true);
            }
            if theme::button(ui, Kind::Secondary, Some(Icon::Close), tr("Cancel", "إلغاء"), true).clicked() {
                decision = Some(false);
            }
        });

        if toggle_skipped {
            self.show_skipped = !self.show_skipped;
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
                        tr("Settings, the undo record and face memory are kept in:", "الإعدادات وسجل التراجع وذاكرة الوجوه محفوظة في:"),
                        13.0,
                        pal().muted,
                        false,
                    );
                    ui.label(theme::muted(self.storage.dir().display().to_string()).small());
                    ui.add_space(8.0);
                    theme::row(ui, |ui| {
                        if theme::button(ui, Kind::Secondary, Some(Icon::External), tr("Open data folder", "فتح مجلد البيانات"), true).clicked() {
                            let dir = self.storage.dir().to_path_buf();
                            let _ = std::fs::create_dir_all(&dir);
                            self.open_in_explorer(&dir);
                        }
                        let size = self.storage.face_cache_size();
                        let label = format!("{} ({:.1} MB)", tr("Clear face memory", "مسح ذاكرة الوجوه"), size as f64 / 1_048_576.0);
                        if theme::button(ui, Kind::Ghost, Some(Icon::Trash), &label, size > 0).clicked() {
                            self.confirm_clear_faces = true;
                        }
                    });
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
                ("photos into each person's folder, recitations and songs into each artist's folder —", "الصور في مجلد كل شخص، والتلاوات والأغاني في مجلد كل قارئ وفنان —"),
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
            ui.label(theme::muted(t("Face models: YuNet (MIT) and SFace (Apache 2.0) from OpenCV Zoo", "نماذج الوجوه: YuNet (MIT) وSFace (Apache 2.0) من OpenCV Zoo")).small());
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
        if self.confirm_clear_faces {
            let body = tr("Photos will be analyzed again next time (a few minutes).", "ستُحلَّل الصور من جديد في المرة القادمة (بضع دقائق).");
            let note = tr("Your photos are not affected.", "صورك لا تتأثر.");
            match self.dialog(ctx, "faces", tr("Clear face memory", "مسح ذاكرة الوجوه"), body, note) {
                Some(true) => {
                    self.confirm_clear_faces = false;
                    self.status_message = match self.storage.clear_face_cache() {
                        Ok(()) => tr("Face memory cleared.", "مُسحت ذاكرة الوجوه.").to_string(),
                        Err(err) => format!("⚠ {}", err),
                    };
                }
                Some(false) => self.confirm_clear_faces = false,
                None => {}
            }
        }
    }
}

/// أيقونة مجلد الوجهة في المعاينة حسب نوعه
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
