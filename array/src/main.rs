#![windows_subsystem = "windows"]

mod faces;
mod grouping;
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
use icons::Icon;
use theme::{pal, Kind};

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
        label: &'static str,
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
        Err(TryRecvError::Disconnected) => Some(Err("the operation stopped unexpectedly".to_string())),
        Err(TryRecvError::Empty) => None,
    }
}

/// اسم المجلد المختار نفسه في ملخص المعاينة
const THIS_FOLDER: &str = "This folder";

/// خطة جاهزة للعرض: النصوص محسوبة مرة واحدة وليس في كل إطار
struct Preview {
    plan: Plan,
    /// (اسم المجلد، عدد الملفات، موجود أم جديد، عدد مجلدات الفنانين)
    destinations: Vec<(String, usize, bool, usize)>,
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
            .map(|m| {
                let mut to = relative(&m.to, &plan.source);
                if let Some((_, score)) = plan.face_matches.get(&m.from) {
                    to.push_str(&format!("   ·  face {:.0}%", score * 100.0));
                }
                (rtl::for_display(&relative(&m.from, &plan.source)), rtl::for_display(&to))
            })
            .collect();

        let skipped = plan
            .skipped
            .iter()
            .map(|(dir, reason)| rtl::for_display(&format!("{}  —  {}", relative(dir, &plan.source), reason)))
            .collect();

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
    /// السمة المطبّقة حالياً (None قبل أول إطار)
    dark: Option<bool>,
}

impl ArrayApp {
    fn new(ctx: &egui::Context, storage: Storage) -> Self {
        let (options, status_message) = match storage.load_settings() {
            Ok(options) => (options, "Choose a folder to start.".to_string()),
            Err(err) => (Options::default(), format!("⚠ {}", err)),
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
                        format!(
                            "Nothing to organize: the loose files are already in place and {} in your existing folders are left as they are.",
                            files(plan.kept_in_folders)
                        )
                    } else if plan.in_place > 0 {
                        format!("This folder is already organized ({} in the right place).", files(plan.in_place))
                    } else {
                        "No files to organize in this folder.".to_string()
                    };
                    if !plan.skipped.is_empty() {
                        message.push_str(&format!(" {} left untouched.", folders(plan.skipped.len())));
                    }
                    self.status_message = message.clone();
                    self.outcome = Some(Outcome { message, folder: plan.source.clone(), ok: true });
                }
                Some(Ok(plan)) => {
                    self.status_message = "Review where each file will go, then confirm.".to_string();
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
        self.status_message = "Ready. Press Preview to see where each file will go.".to_string();
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
        self.status_message = "⏳ Scanning files...".to_string();
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
                .map_err(|e| format!("{} Nothing was moved.", e))?;
            let result = OrganizerEngine::execute(&plan, &counter);
            // السجل يحتفظ فقط بما نُقل فعلاً
            let saved = if result.done.is_empty() {
                storage.clear_undo()
            } else {
                storage.save_undo(&UndoLog { source: plan.source.clone(), moves: result.done.clone(), created_dirs: plan.new_dirs.clone() })
            };

            let mut msg = format!("✔ Organized {} in {}", files(result.done.len()), folder_name(&plan.source));
            if !result.failed.is_empty() {
                msg.push_str(&format!(
                    "\n⚠ {} could not be moved (in use, access denied, or a file with the same name appeared) and were left in place.",
                    files(result.failed.len())
                ));
            }
            if let Err(e) = saved {
                msg.push_str(&format!("\n⚠ {}", e));
            }
            Ok(msg)
        });
        self.job = Some(Job::Moving { label: "Organizing", rx, progress, total, folder });
        self.status_message = "⏳ Organizing... please wait".to_string();
    }

    fn start_undo(&mut self, ctx: &egui::Context) {
        let progress = Arc::new(AtomicUsize::new(0));
        let total = self.undo_entries;
        let folder = self.undo_source.clone().unwrap_or_default();
        let storage = self.storage.clone();
        let counter = progress.clone();
        let rx = spawn_job(ctx, move || {
            let Some(log) = storage.load_undo()? else {
                return Err("There is nothing to undo.".to_string());
            };
            let result = OrganizerEngine::undo(&log.moves, &log.source, &log.created_dirs, &counter);
            // الملفات التي لم تُرجع تبقى في السجل لإعادة المحاولة
            if result.failed.is_empty() {
                storage.clear_undo()?;
            } else {
                storage.save_undo(&UndoLog { source: log.source.clone(), moves: result.failed.clone(), created_dirs: log.created_dirs.clone() })?;
            }

            let mut msg = format!("↩ Restored {} to their original places in {}", files(result.done.len()), folder_name(&log.source));
            if !result.failed.is_empty() {
                msg.push_str(&format!(
                    "\n⚠ {} could not be restored (moved, renamed, in use, or a new file has the same name). Press Undo again to retry them.",
                    files(result.failed.len())
                ));
            }
            Ok(msg)
        });
        self.job = Some(Job::Moving { label: "Restoring", rx, progress, total, folder });
        self.page = Page::Organize;
        self.status_message = "⏳ Restoring files... please wait".to_string();
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
                self.status_message = format!("⚠ {} This choice is used for now but not saved.", err);
            }
        }
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
            Ok(_) => "Edit the lists in Notepad and save. They are used from the next preview.".to_string(),
            Err(err) => format!("⚠ Could not open {}: {}", path.display(), err),
        };
    }

    fn open_in_explorer(&mut self, path: &Path) {
        if let Err(err) = std::process::Command::new("explorer").arg(path).spawn() {
            self.status_message = format!("⚠ Could not open {}: {}", path.display(), err);
        }
    }

    // ---------------- الواجهة ----------------

    fn sidebar(&mut self, ctx: &egui::Context) {
        let p = pal();
        egui::SidePanel::left("nav")
            .exact_width(228.0)
            .resizable(false)
            .frame(egui::Frame::none().fill(p.sidebar).inner_margin(egui::Margin::symmetric(14.0, 22.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    if let Some(logo) = &self.logo {
                        ui.add(egui::Image::from_texture(egui::load::SizedTexture::new(logo.id(), [42.0, 42.0])));
                    }
                    ui.vertical(|ui| {
                        ui.label(theme::strong("Array", 21.0));
                        ui.label(theme::muted("File organizer").small());
                    });
                });
                ui.add_space(28.0);

                let busy = self.busy();
                for (page, icon, name) in [
                    (Page::Organize, Icon::Organize, "Organize"),
                    (Page::History, Icon::History, "History"),
                    (Page::Settings, Icon::Settings, "Settings"),
                    (Page::About, Icon::About, "About"),
                ] {
                    // أثناء المعاينة أو العمل تبقى صفحة التنظيم ظاهرة
                    if theme::nav_item(ui, icon, name, self.page == page, !busy || page == Page::Organize).clicked() {
                        self.page = page;
                    }
                    ui.add_space(2.0);
                }

                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.label(theme::muted(format!("Version {}", VERSION)).small());
                    theme::icon_text(ui, Icon::Lock, theme::muted("Works offline on this PC").small(), p.muted);
                });
            });
    }

    fn page_header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
        ui.add_space(4.0);
        ui.label(theme::strong(title, 28.0));
        ui.label(theme::muted(subtitle));
        ui.add_space(16.0);
    }

    fn organize_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.preview.is_some() {
            self.preview_view(ui, ctx);
            return;
        }
        if self.job.is_some() {
            Self::page_header(ui, "Organizing", "Working on it — you can keep using your computer.");
            self.progress_view(ui);
            return;
        }

        Self::page_header(ui, "Organize a folder", "Sort loose photos, music, videos and documents into tidy folders. Nothing moves until you confirm.");
        let p = pal();

        egui::ScrollArea::vertical().auto_shrink([false, false]).max_height((ui.available_height() - 60.0).max(120.0)).show(ui, |ui| {
            if let Some(outcome) = self.outcome.take() {
                if self.outcome_view(ui, &outcome) {
                    self.outcome = Some(outcome);
                }
                ui.add_space(14.0);
            }

            // الخطوة 1: المجلد
            theme::card(ui, |ui| {
                theme::step_title(ui, "1", "Choose a folder");
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
                                ui.horizontal(|ui| {
                                    icons::show(ui, Icon::Folder, 34.0, p.accent);
                                    ui.vertical(|ui| {
                                        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string());
                                        ui.label(theme::strong(rtl::for_display(&name), 18.0));
                                        ui.label(theme::muted(rtl::for_display(&path.display().to_string())).small());
                                    });
                                });
                                ui.add_space(10.0);
                                ui.horizontal(|ui| {
                                    if theme::button(ui, Kind::Secondary, Some(Icon::Folder), "Change folder", true).clicked() {
                                        if let Some(picked) = FileDialog::new().pick_folder() {
                                            self.select_folder(picked);
                                        }
                                    }
                                    if theme::button(ui, Kind::Ghost, Some(Icon::External), "Open in Explorer", true).clicked() {
                                        self.open_in_explorer(&path);
                                    }
                                });
                            }
                            None => {
                                ui.vertical_centered(|ui| {
                                    icons::show(ui, Icon::Drop, 40.0, if hovering { p.accent } else { p.muted });
                                    ui.add_space(4.0);
                                    ui.label(theme::strong(if hovering { "Release to choose this folder" } else { "Drop a folder here" }, 17.0));
                                    ui.label(theme::muted("or"));
                                    if theme::button(ui, Kind::Secondary, Some(Icon::Folder), "Browse…", true).clicked() {
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
                theme::step_title(ui, "2", "How to organize");
                ui.add_space(10.0);
                self.option_toggles(ui, true);
            });
            ui.add_space(8.0);
        });

        // الخطوة 3: زر المعاينة مثبّت أسفل الصفحة فيبقى ظاهراً دائماً
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let can_start = self.selected_path.is_some();
            if theme::button(ui, Kind::Primary, Some(Icon::Search), "Preview changes", can_start).clicked() {
                if let Some(path) = self.selected_path.clone() {
                    self.start_planning(ctx, path);
                }
            }
            ui.add_space(6.0);
            ui.label(theme::muted(if can_start { "You will see every change before anything moves." } else { "Choose a folder first." }));
        });
    }

    /// مفاتيح الخيارات الثلاثة (في صفحة التنظيم بشرح مفصّل، وفي الإعدادات مختصرة)
    fn option_toggles(&mut self, ui: &mut egui::Ui, detailed: bool) {
        let mut keep = self.options.keep_existing_folders;
        let help = if detailed {
            "Files already inside subfolders are not moved or renamed. Turn off to empty and rebuild all subfolders."
        } else {
            "Only loose files are organized."
        };
        if theme::toggle_row(ui, true, &mut keep, "Keep my existing folders as they are", help) {
            self.set_option(|o| o.keep_existing_folders = keep);
        }
        ui.add_space(8.0);
        let mut by_faces = self.options.sort_by_faces;
        let help = if detailed {
            "Learns faces from folders like Ali or Mom and suggests a photo only when one known person is clearly recognized."
        } else {
            "Needs 'Keep my existing folders'."
        };
        if theme::toggle_row(ui, keep, &mut by_faces, "Sort loose photos into people folders by face", help) {
            self.set_option(|o| o.sort_by_faces = by_faces);
        }
        ui.add_space(8.0);
        let mut rename = self.options.rename_files;
        let help = if detailed {
            "Removes codes like [AUD-…] and (1), fixes garbled text, and names files like 4749d951.mp3 from their context."
        } else {
            "Off: files keep their original names."
        };
        if theme::toggle_row(ui, true, &mut rename, "Clean up file names", help) {
            self.set_option(|o| o.rename_files = rename);
        }
    }

    /// بطاقة نتيجة آخر عملية. ترجع false عند إغلاقها
    fn outcome_view(&mut self, ui: &mut egui::Ui, outcome: &Outcome) -> bool {
        let p = pal();
        let mut keep = true;
        theme::card(ui, |ui| {
            let (icon, color) = if outcome.ok { (Icon::Check, p.success) } else { (Icon::Warning, p.danger) };
            ui.horizontal(|ui| {
                // دائرة خلف الأيقونة بلون النتيجة
                let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 20.0, color.gamma_multiply(0.18));
                icons::paint(ui.painter(), rect.shrink(9.0), icon, color, 0.0);
                ui.vertical(|ui| {
                    for (i, line) in outcome.message.lines().enumerate() {
                        // الرمز في بداية السطر يظهر مرة واحدة في الأيقونة
                        let line = line.trim_start_matches(['✔', '↩', '⚠', '❌']).trim_start();
                        if i == 0 {
                            ui.label(theme::strong(rtl::for_display(line), 16.0));
                            if outcome.folder.exists() {
                                ui.label(theme::muted(rtl::for_display(&outcome.folder.display().to_string())).small());
                            }
                        } else {
                            theme::icon_text(ui, Icon::Warning, RichText::new(rtl::for_display(line)).color(p.amber), p.amber);
                        }
                    }
                });
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if outcome.folder.exists() && theme::button(ui, Kind::Secondary, Some(Icon::External), "Open folder", true).clicked() {
                    self.open_in_explorer(&outcome.folder.clone());
                }
                if let Some(n) = self.undo_count {
                    if theme::button(ui, Kind::Secondary, Some(Icon::Undo), &format!("Undo ({})", files(n)), true).clicked() {
                        self.confirm_undo = true;
                    }
                }
                if theme::button(ui, Kind::Ghost, Some(Icon::Folder), "Organize another folder", true).clicked() {
                    self.selected_path = None;
                    keep = false;
                }
                if theme::button(ui, Kind::Ghost, Some(Icon::Close), "Close", true).clicked() {
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
                ui.horizontal(|ui| {
                    icons::spinner(ui, 22.0, p.accent);
                    ui.label(theme::strong(&stage, 17.0));
                });
                ui.add_space(8.0);
                ui.add(egui::ProgressBar::new(done as f32 / total as f32).text(format!("{} / {} photos", done, total)).fill(p.primary));
                ui.add_space(4.0);
                theme::icon_text(ui, Icon::Person, theme::muted("The first time takes a few minutes; faces are remembered for next time.").small(), p.muted);
            }
            Some(Job::Planning(..)) => {
                ui.horizontal(|ui| {
                    icons::spinner(ui, 22.0, p.accent);
                    ui.label(theme::strong("Scanning files…", 17.0));
                });
            }
            Some(Job::Moving { label, progress, total, .. }) => {
                let done = progress.load(Ordering::Relaxed);
                let fraction = if *total == 0 { 1.0 } else { done as f32 / *total as f32 };
                ui.horizontal(|ui| {
                    icons::spinner(ui, 22.0, p.accent);
                    ui.label(theme::strong(format!("{}…", label), 17.0));
                });
                ui.add_space(8.0);
                ui.add(egui::ProgressBar::new(fraction).text(format!("{} / {} files", done, total)).fill(p.primary));
            }
            None => {}
        });
    }

    fn preview_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(preview) = &self.preview else { return };
        let p = pal();
        let mut decision: Option<bool> = None;
        let plan = &preview.plan;

        Self::page_header(ui, "Review the changes", "Nothing has moved yet. Check the list, then organize — you can undo it later.");

        // الأرقام الأساسية
        ui.horizontal(|ui| {
            // عرض كل مربع = (المتاح - الهوامش والحدود 4×30 - المسافات 3×10) ÷ 4
            let width = ((ui.available_width() - 4.0 * 30.0 - 3.0 * 10.0) / 4.0).max(100.0);
            theme::stat(ui, width, Icon::ArrowRight, plan.moves.len(), "to move or rename", p.accent);
            theme::stat(ui, width, Icon::Check, plan.kept_in_folders + plan.in_place, "stay where they are", p.text);
            theme::stat(ui, width, Icon::Person, plan.face_matches.len(), "matched by face", p.amber);
            theme::stat(ui, width, Icon::Shield, plan.skipped.len(), "protected folders", p.success);
        });
        ui.add_space(12.0);

        // إلى أين تذهب الملفات
        ui.horizontal_wrapped(|ui| {
            for (folder, n, existing, artists) in &preview.destinations {
                let label = if folder == THIS_FOLDER {
                    format!("{}  ·  {}", THIS_FOLDER, n)
                } else if *artists > 0 {
                    format!("{}  ·  {}  ·  {} folder{} inside", folder, n, artists, if *artists == 1 { "" } else { "s" })
                } else {
                    format!("{}  ·  {}{}", folder, n, if *existing { "" } else { "  ·  new" })
                };
                theme::chip(ui, category_icon(folder), &rtl::for_display(&label));
            }
        });
        ui.add_space(4.0);
        if !plan.people.is_empty() {
            theme::icon_text(ui, Icon::Person, theme::muted(rtl::for_display(&format!("People learned from your folders: {}", plan.people.join(", ")))), p.muted);
        }
        if let Some(note) = &plan.face_note {
            theme::icon_text(ui, Icon::About, theme::muted(note.as_str()).small(), p.muted);
        }
        if !preview.skipped.is_empty() {
            egui::CollapsingHeader::new(RichText::new(format!("{} will not be touched", folders(preview.skipped.len()))).color(p.success))
                .id_source("skipped_folders")
                .icon(|ui, openness, response| {
                    let rect = egui::Rect::from_center_size(response.rect.center(), egui::vec2(16.0, 16.0));
                    icons::paint(ui.painter(), rect, Icon::Shield, pal().success.gamma_multiply(0.6 + 0.4 * openness), 0.0);
                })
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().id_source("skipped_list").max_height(90.0).show(ui, |ui| {
                        for line in &preview.skipped {
                            ui.label(RichText::new(line).small());
                        }
                    });
                });
        }
        ui.add_space(10.0);

        // الجدول
        let row_height = ui.text_style_height(&egui::TextStyle::Body) + 12.0;
        let table_height = (ui.available_height() - 70.0).max(row_height * 4.0);
        theme::card(ui, |ui| {
            let column = ((ui.available_width() - 100.0) / 2.0).max(60.0);
            ui.horizontal(|ui| {
                table_cell(ui, column, 20.0, theme::muted("CURRENT LOCATION").small());
                ui.add_space(20.0);
                table_cell(ui, column, 20.0, theme::muted("NEW LOCATION").small());
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
                        ui.horizontal(|ui| {
                            table_cell(ui, column, row_height, theme::muted(from.as_str()));
                            icons::show(ui, Icon::ArrowRight, 16.0, p.accent);
                            table_cell(ui, column, row_height, RichText::new(to.as_str()));
                        });
                    }
                });
        });
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            if theme::button(ui, Kind::Primary, Some(Icon::Check), &format!("Organize {}", files(preview.rows.len())), true).clicked() {
                decision = Some(true);
            }
            if theme::button(ui, Kind::Secondary, Some(Icon::Close), "Cancel", true).clicked() {
                decision = Some(false);
            }
        });

        match decision {
            Some(true) => {
                if let Some(preview) = self.preview.take() {
                    self.start_organizing(ctx, preview.plan);
                }
            }
            Some(false) => {
                self.preview = None;
                self.status_message = "Cancelled. No files were moved.".to_string();
            }
            None => {}
        }
    }

    fn history_page(&mut self, ui: &mut egui::Ui) {
        let p = pal();
        Self::page_header(ui, "History", "Undo the last organize and put every file back where it was.");
        let last = self.undo_source.clone().zip(self.undo_count);
        theme::card(ui, |ui| match last {
            Some((source, count)) => {
                theme::section_title(ui, Icon::History, "Last organize");
                ui.add_space(8.0);
                theme::icon_text(ui, Icon::Folder, theme::strong(rtl::for_display(&folder_name(&source)), 15.0), p.muted);
                ui.label(theme::muted(rtl::for_display(&source.display().to_string())).small());
                if let Some(when) = self.storage.undo_saved_at() {
                    let local: DateTime<Local> = when.into();
                    theme::icon_text(ui, Icon::Clock, theme::muted(local.format("%Y-%m-%d  %H:%M").to_string()), p.muted);
                }
                theme::icon_text(ui, Icon::Undo, theme::muted(format!("{} can be moved back", files(count))), p.muted);
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if theme::button(ui, Kind::Primary, Some(Icon::Undo), "Undo last organize", true).clicked() {
                        self.confirm_undo = true;
                    }
                    if theme::button(ui, Kind::Secondary, Some(Icon::External), "Open folder", true).clicked() {
                        self.open_in_explorer(&source);
                    }
                });
                ui.add_space(6.0);
                ui.label(theme::muted("Files you changed or replaced since then are left alone.").small());
            }
            None => {
                ui.vertical_centered(|ui| {
                    ui.add_space(10.0);
                    icons::show(ui, Icon::History, 40.0, p.muted);
                    ui.label(theme::strong("Nothing to undo", 17.0));
                    ui.label(theme::muted("After you organize a folder, you can put everything back from here."));
                    ui.add_space(10.0);
                });
            }
        });
    }

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        Self::page_header(ui, "Settings", "Saved automatically and used from the next preview.");
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            theme::card(ui, |ui| {
                theme::section_title(ui, Icon::Settings, "Appearance");
                ui.add_space(8.0);
                let mut selected = match self.options.appearance.as_str() {
                    "light" => 1,
                    "dark" => 2,
                    _ => 0,
                };
                if theme::segmented(ui, &["System", "Light", "Dark"], &mut selected) {
                    let value = ["system", "light", "dark"][selected].to_string();
                    self.set_option(move |o| o.appearance = value.clone());
                }
            });
            ui.add_space(14.0);

            theme::card(ui, |ui| {
                theme::section_title(ui, Icon::Organize, "Organizing");
                ui.add_space(8.0);
                self.option_toggles(ui, false);
            });
            ui.add_space(14.0);

            theme::card(ui, |ui| {
                theme::section_title(ui, Icon::Music, "Music grouping");
                ui.label(theme::muted("Words that mark a speaker in a file name, and names of AI music tools."));
                ui.add_space(8.0);
                ui.label(RichText::new(rtl::for_display(&format!("Speaker words:  {}", self.options.speaker_keywords.join("، ")))).small());
                ui.label(RichText::new(rtl::for_display(&format!("AI tools:  {}", self.options.ai_markers.join(", ")))).small());
                ui.add_space(8.0);
                if theme::button(ui, Kind::Secondary, Some(Icon::Edit), "Edit lists in Notepad", true).clicked() {
                    self.open_settings_file();
                }
            });
            ui.add_space(14.0);

            theme::card(ui, |ui| {
                theme::section_title(ui, Icon::Lock, "Data on this PC");
                ui.label(theme::muted(rtl::for_display(&format!("Settings, the undo record and face memory are kept in {}", self.storage.dir().display()))).small());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if theme::button(ui, Kind::Secondary, Some(Icon::External), "Open data folder", true).clicked() {
                        let dir = self.storage.dir().to_path_buf();
                        let _ = std::fs::create_dir_all(&dir);
                        self.open_in_explorer(&dir);
                    }
                    let size = self.storage.face_cache_size();
                    let label = format!("Clear face memory ({:.1} MB)", size as f64 / 1_048_576.0);
                    if theme::button(ui, Kind::Ghost, Some(Icon::Trash), &label, size > 0).clicked() {
                        self.confirm_clear_faces = true;
                    }
                });
            });
        });
    }

    fn about_page(&self, ui: &mut egui::Ui) {
        let p = pal();
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            if let Some(logo) = &self.logo {
                ui.add(egui::Image::from_texture(egui::load::SizedTexture::new(logo.id(), [112.0, 112.0])));
            }
            ui.add_space(6.0);
            ui.label(theme::strong("Array", 32.0));
            ui.label(theme::muted(format!("Version {}", VERSION)));
            ui.add_space(14.0);
            ui.label("Gathers scattered files and puts them in tidy folders —");
            ui.label("photos into each person's folder, recitations and songs into each artist's folder —");
            ui.label("without touching what you already organized.");
            ui.add_space(18.0);
            // سطر بأيقونة في المنتصف: عرضه يُحسب ثم تُترك مسافة مساوية على الجانبين
            let note = "Everything runs on this computer. Nothing is uploaded.";
            let width = ui.painter().layout_no_wrap(note.to_owned(), egui::FontId::proportional(15.0), p.success).size().x + 26.0;
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                icons::show(ui, Icon::Lock, 16.0, p.success);
                ui.label(RichText::new(note).color(p.success));
            });
            ui.add_space(18.0);
            ui.label(theme::muted("Face models: YuNet (MIT) and SFace (Apache 2.0) from OpenCV Zoo").small());
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let p = pal();
        if self.confirm_undo {
            let count = self.undo_count.unwrap_or(0);
            egui::Window::new(theme::strong("Undo last organize", 17.0))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("Move {} back to where they were before the last organize?", files(count)));
                    ui.label(theme::muted("Files that were changed or replaced since then are left alone.").small());
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if theme::button(ui, Kind::Primary, Some(Icon::Undo), "Yes, undo", true).clicked() {
                            self.confirm_undo = false;
                            self.start_undo(ctx);
                        }
                        if theme::button(ui, Kind::Secondary, None, "Cancel", true).clicked() {
                            self.confirm_undo = false;
                        }
                    });
                });
        }
        if self.confirm_clear_faces {
            egui::Window::new(theme::strong("Clear face memory", 17.0))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Photos will be analyzed again next time (a few minutes).");
                    ui.label(RichText::new("Your photos are not affected.").small().color(p.muted));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if theme::button(ui, Kind::Primary, Some(Icon::Trash), "Clear", true).clicked() {
                            self.confirm_clear_faces = false;
                            self.status_message = match self.storage.clear_face_cache() {
                                Ok(()) => "Face memory cleared.".to_string(),
                                Err(err) => format!("⚠ {}", err),
                            };
                        }
                        if theme::button(ui, Kind::Secondary, None, "Cancel", true).clicked() {
                            self.confirm_clear_faces = false;
                        }
                    });
                });
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

fn files(n: usize) -> String {
    if n == 1 { "1 file".to_string() } else { format!("{} files", n) }
}

fn folders(n: usize) -> String {
    if n == 1 { "1 program or project folder".to_string() } else { format!("{} program or project folders", n) }
}

/// خلية بعرض ثابت؛ الاسم الطويل يُختصر ويظهر كاملاً عند المرور عليه
fn table_cell(ui: &mut egui::Ui, width: f32, height: f32, text: RichText) {
    ui.allocate_ui_with_layout(egui::vec2(width, height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(width);
        let full = text.text().to_string();
        ui.add(egui::Label::new(text).truncate(true)).on_hover_text(full);
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
                ui.horizontal(|ui| {
                    if let Some(icon) = icon {
                        icons::show(ui, icon, 14.0, color);
                    }
                    ui.add(egui::Label::new(theme::muted(rtl::for_display(text.trim_start())).small()).truncate(true));
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(p.bg).inner_margin(egui::Margin::symmetric(32.0, 24.0)))
            .show(ctx, |ui| match self.page {
                Page::Organize => self.organize_page(ui, ctx),
                Page::History => self.history_page(ui),
                Page::Settings => self.settings_page(ui),
                Page::About => self.about_page(ui),
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
