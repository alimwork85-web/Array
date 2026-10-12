#![windows_subsystem = "windows"]

mod faces;
mod grouping;
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
use theme::{ACCENT, AMBER, GREEN, MUTED, RED};

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
        ..Default::default()
    };

    eframe::run_native(
        "Array",
        options,
        Box::new(|cc| {
            rtl::install_arabic_font(&cc.egui_ctx);
            theme::apply(&cc.egui_ctx);
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

            let mut msg = format!("✔ Organized {} in {}", files(result.done.len()), plan.source.display());
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

            let mut msg = format!("↩ Restored {} to their original places in {}", files(result.done.len()), log.source.display());
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
        egui::SidePanel::left("nav")
            .exact_width(220.0)
            .resizable(false)
            .frame(egui::Frame::none().fill(theme::SIDEBAR).inner_margin(egui::Margin::symmetric(16.0, 20.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if let Some(logo) = &self.logo {
                        ui.add(egui::Image::from_texture(egui::load::SizedTexture::new(logo.id(), [44.0, 44.0])));
                    }
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Array").size(22.0).strong());
                        ui.label(RichText::new("File organizer").small().color(MUTED));
                    });
                });
                ui.add_space(26.0);

                let busy = self.busy();
                for (page, icon, name) in [
                    (Page::Organize, "📂", "Organize"),
                    (Page::History, "↩", "History"),
                    (Page::Settings, "⚙", "Settings"),
                    (Page::About, "ℹ", "About"),
                ] {
                    let selected = self.page == page;
                    let text = RichText::new(format!("{}   {}", icon, name)).size(16.0).color(if selected { egui::Color32::WHITE } else { theme::TEXT });
                    let button = egui::Button::new(text)
                        .fill(if selected { theme::ACCENT_DARK } else { egui::Color32::TRANSPARENT })
                        .min_size(egui::vec2(ui.available_width(), 40.0));
                    // أثناء المعاينة أو العمل تبقى صفحة التنظيم ظاهرة
                    if ui.add_enabled(!busy || page == Page::Organize, button).clicked() {
                        self.page = page;
                    }
                }

                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.label(RichText::new(format!("Version {}", VERSION)).small().color(MUTED));
                    ui.label(RichText::new("🔒 Works offline on this PC").small().color(MUTED));
                });
            });
    }

    fn page_header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
        ui.add_space(6.0);
        ui.label(RichText::new(title).size(26.0).strong());
        ui.label(RichText::new(subtitle).color(MUTED));
        ui.add_space(14.0);
    }

    fn organize_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.preview.is_some() {
            self.preview_view(ui, ctx);
            return;
        }
        if self.job.is_some() {
            Self::page_header(ui, "Organize", "Working on it — you can keep using your computer.");
            self.progress_view(ui);
            return;
        }

        Self::page_header(ui, "Organize a folder", "Sort loose photos, music, videos and documents into tidy folders. Nothing moves until you confirm.");

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if let Some(outcome) = self.outcome.take() {
                let keep = self.outcome_view(ui, ctx, &outcome);
                if keep {
                    self.outcome = Some(outcome);
                }
                ui.add_space(12.0);
            }

            // الخطوة 1: المجلد
            theme::card(ui, |ui| {
                theme::step_title(ui, "1", "Choose a folder");
                ui.add_space(8.0);
                let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
                let drop_fill = if hovering { theme::ACCENT_DARK } else { egui::Color32::from_rgb(20, 27, 37) };
                egui::Frame::none()
                    .fill(drop_fill)
                    .stroke(egui::Stroke::new(1.5_f32, if hovering { ACCENT } else { theme::CARD_BORDER }))
                    .rounding(10.0)
                    .inner_margin(egui::Margin::same(18.0))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        match self.selected_path.clone() {
                            Some(path) => {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("📁").size(30.0));
                                    ui.vertical(|ui| {
                                        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string());
                                        ui.label(RichText::new(rtl::for_display(&name)).size(18.0).strong());
                                        ui.label(RichText::new(rtl::for_display(&path.display().to_string())).small().color(MUTED));
                                    });
                                });
                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    if ui.button("Change folder…").clicked() {
                                        if let Some(p) = FileDialog::new().pick_folder() {
                                            self.select_folder(p);
                                        }
                                    }
                                    if ui.button("Open in Explorer").clicked() {
                                        self.open_in_explorer(&path);
                                    }
                                });
                            }
                            None => {
                                ui.vertical_centered(|ui| {
                                    ui.label(RichText::new("📂").size(40.0));
                                    ui.label(RichText::new("Drop a folder here").size(17.0).strong());
                                    ui.label(RichText::new("or").color(MUTED));
                                    if ui.button(RichText::new("Browse…").strong()).clicked() {
                                        if let Some(p) = FileDialog::new().pick_folder() {
                                            self.select_folder(p);
                                        }
                                    }
                                });
                            }
                        }
                    });
            });
            ui.add_space(12.0);

            // الخطوة 2: الخيارات
            theme::card(ui, |ui| {
                theme::step_title(ui, "2", "How to organize");
                ui.add_space(8.0);
                let mut keep = self.options.keep_existing_folders;
                if theme::toggle_row(
                    ui,
                    true,
                    &mut keep,
                    "Keep my existing folders as they are",
                    "Files already inside subfolders are not moved or renamed. Turn off to empty and rebuild all subfolders.",
                ) {
                    self.set_option(|o| o.keep_existing_folders = keep);
                }
                ui.add_space(6.0);
                let mut by_faces = self.options.sort_by_faces;
                if theme::toggle_row(
                    ui,
                    keep,
                    &mut by_faces,
                    "Sort loose photos into people folders by face",
                    "Learns faces from folders like Ali or Mom and suggests a photo only when one known person is clearly recognized.",
                ) {
                    self.set_option(|o| o.sort_by_faces = by_faces);
                }
                ui.add_space(6.0);
                let mut rename = self.options.rename_files;
                if theme::toggle_row(
                    ui,
                    true,
                    &mut rename,
                    "Clean up file names",
                    "Removes codes like [AUD-…] and (1), fixes garbled text, and names files like 4749d951.mp3 from their context.",
                ) {
                    self.set_option(|o| o.rename_files = rename);
                }
            });
            ui.add_space(16.0);

            // الخطوة 3: المعاينة
            ui.horizontal(|ui| {
                let can_start = self.selected_path.is_some();
                if theme::primary_button(ui, can_start, "🔍  Preview changes").clicked() {
                    if let Some(path) = self.selected_path.clone() {
                        self.start_planning(ctx, path);
                    }
                }
                ui.label(RichText::new("You will see every change before anything moves.").color(MUTED));
            });
        });
    }

    /// بطاقة نتيجة آخر عملية. ترجع false عند إغلاقها
    fn outcome_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, outcome: &Outcome) -> bool {
        let mut keep = true;
        theme::card(ui, |ui| {
            let (icon, color) = if outcome.ok { ("✔", GREEN) } else { ("⚠", RED) };
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon).size(26.0).color(color));
                ui.vertical(|ui| {
                    for (i, line) in outcome.message.lines().enumerate() {
                        // الرمز في بداية السطر يظهر مرة واحدة في الأيقونة الكبيرة
                        let line = line.trim_start_matches(['✔', '↩', '⚠', '❌']).trim_start();
                        let text = RichText::new(rtl::for_display(line));
                        ui.label(if i == 0 { text.size(16.0).strong() } else { text.color(AMBER) });
                    }
                });
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if outcome.folder.exists() && ui.button("📁  Open folder").clicked() {
                    self.open_in_explorer(&outcome.folder.clone());
                }
                if self.undo_count.is_some() && ui.button(format!("↩  Undo ({})", files(self.undo_count.unwrap_or(0)))).clicked() {
                    self.confirm_undo = true;
                }
                if ui.button("Organize another folder").clicked() {
                    self.selected_path = None;
                    keep = false;
                }
                if ui.button("Close").clicked() {
                    keep = false;
                }
            });
            let _ = ctx;
        });
        keep
    }

    fn progress_view(&self, ui: &mut egui::Ui) {
        theme::card(ui, |ui| match &self.job {
            Some(Job::Planning(_, progress)) if progress.total.load(Ordering::Relaxed) > 0 => {
                let (done, total) = (progress.done.load(Ordering::Relaxed), progress.total.load(Ordering::Relaxed));
                let stage = progress.stage.lock().unwrap().clone();
                ui.label(RichText::new(&stage).size(17.0).strong());
                ui.add_space(6.0);
                ui.add(egui::ProgressBar::new(done as f32 / total as f32).text(format!("{} / {} photos", done, total)).fill(ACCENT));
                ui.label(RichText::new("The first time takes a few minutes; faces are remembered for next time.").small().color(MUTED));
            }
            Some(Job::Planning(..)) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Scanning files…").size(17.0).strong());
                });
            }
            Some(Job::Moving { label, progress, total, .. }) => {
                let done = progress.load(Ordering::Relaxed);
                let fraction = if *total == 0 { 1.0 } else { done as f32 / *total as f32 };
                ui.label(RichText::new(format!("{}…", label)).size(17.0).strong());
                ui.add_space(6.0);
                ui.add(egui::ProgressBar::new(fraction).text(format!("{} / {} files", done, total)).fill(ACCENT));
            }
            None => {}
        });
    }

    fn preview_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(preview) = &self.preview else { return };
        let mut decision: Option<bool> = None;
        let plan = &preview.plan;

        Self::page_header(ui, "Review the changes", "Nothing has moved yet. Check the list, then organize — you can undo it later.");

        // الأرقام الأساسية
        ui.horizontal(|ui| {
            // عرض كل مربع = (المتاح - هوامش المربعات الداخلية 4×28 - المسافات 3×10) ÷ 4
            let width = ((ui.available_width() - 4.0 * 28.0 - 3.0 * 10.0) / 4.0).max(100.0);
            theme::stat(ui, width, plan.moves.len(), "to move or rename", ACCENT);
            theme::stat(ui, width, plan.kept_in_folders + plan.in_place, "stay where they are", theme::TEXT);
            theme::stat(ui, width, plan.face_matches.len(), "matched by face", AMBER);
            theme::stat(ui, width, plan.skipped.len(), "protected folders", GREEN);
        });
        ui.add_space(10.0);

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
                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(30, 40, 54))
                    .rounding(14.0)
                    .inner_margin(egui::Margin::symmetric(10.0, 4.0))
                    .show(ui, |ui| {
                        ui.label(RichText::new(rtl::for_display(&label)).small());
                    });
            }
        });
        if !plan.people.is_empty() {
            ui.label(RichText::new(rtl::for_display(&format!("👤 People learned from your folders: {}", plan.people.join(", ")))).color(MUTED));
        }
        if let Some(note) = &plan.face_note {
            ui.label(RichText::new(format!("ℹ {}", note)).small().color(MUTED));
        }
        if !preview.skipped.is_empty() {
            egui::CollapsingHeader::new(format!("🛡 {} will not be touched", folders(preview.skipped.len())))
                .id_source("skipped_folders")
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().id_source("skipped_list").max_height(90.0).show(ui, |ui| {
                        for line in &preview.skipped {
                            ui.label(RichText::new(line).small());
                        }
                    });
                });
        }
        ui.add_space(8.0);

        // الجدول
        let row_height = ui.text_style_height(&egui::TextStyle::Body) + 10.0;
        let table_height = (ui.available_height() - 64.0).max(row_height * 4.0);
        theme::card(ui, |ui| {
            // عمودان + سهم بينهما + المسافات
            let column = ((ui.available_width() - 100.0) / 2.0).max(60.0);
            ui.horizontal(|ui| {
                table_cell(ui, column, 20.0, RichText::new("Current location").small().strong().color(MUTED));
                ui.label("  ");
                table_cell(ui, column, 20.0, RichText::new("New location").small().strong().color(MUTED));
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(table_height - 50.0)
                .auto_shrink([false, false])
                .show_rows(ui, row_height, preview.rows.len(), |ui, range| {
                    let start = range.start;
                    for (i, (from, to)) in preview.rows[range].iter().enumerate() {
                        let rect = ui.available_rect_before_wrap();
                        if (start + i) % 2 == 1 {
                            ui.painter().rect_filled(
                                egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), row_height)),
                                4.0,
                                egui::Color32::from_rgb(30, 39, 52),
                            );
                        }
                        ui.horizontal(|ui| {
                            table_cell(ui, column, row_height, RichText::new(from.as_str()).color(MUTED));
                            ui.label(RichText::new("→").color(ACCENT));
                            table_cell(ui, column, row_height, RichText::new(to.as_str()));
                        });
                    }
                });
        });
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            if theme::primary_button(ui, true, &format!("✔  Organize {}", files(preview.rows.len()))).clicked() {
                decision = Some(true);
            }
            if ui.add(egui::Button::new(RichText::new("Cancel").size(15.0)).min_size(egui::vec2(120.0, 42.0))).clicked() {
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
        Self::page_header(ui, "History", "Undo the last organize and put every file back where it was.");
        let last = self.undo_source.clone().zip(self.undo_count);
        theme::card(ui, |ui| match last {
            Some((source, count)) => {
                ui.label(RichText::new("Last organize").size(17.0).strong());
                ui.add_space(4.0);
                ui.label(rtl::for_display(&format!("📁  {}", source.display())));
                if let Some(when) = self.storage.undo_saved_at() {
                    let local: DateTime<Local> = when.into();
                    ui.label(RichText::new(format!("🕒  {}", local.format("%Y-%m-%d %H:%M"))).color(MUTED));
                }
                ui.label(RichText::new(format!("{} can be moved back", files(count))).color(MUTED));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if theme::primary_button(ui, true, "↩  Undo last organize").clicked() {
                        self.confirm_undo = true;
                    }
                    if ui.button("Open folder").clicked() {
                        self.open_in_explorer(&source);
                    }
                });
                ui.add_space(4.0);
                ui.label(RichText::new("Files you changed or replaced since then are left alone.").small().color(MUTED));
            }
            _ => {
                ui.label(RichText::new("Nothing to undo").size(17.0).strong());
                ui.label(RichText::new("After you organize a folder, you can put everything back from here.").color(MUTED));
            }
        });
    }

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        Self::page_header(ui, "Settings", "Saved automatically and used from the next preview.");
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            theme::card(ui, |ui| {
                ui.label(RichText::new("Organizing").size(17.0).strong());
                ui.add_space(6.0);
                let mut keep = self.options.keep_existing_folders;
                if theme::toggle_row(ui, true, &mut keep, "Keep my existing folders as they are", "Only loose files are organized.") {
                    self.set_option(|o| o.keep_existing_folders = keep);
                }
                let mut by_faces = self.options.sort_by_faces;
                if theme::toggle_row(ui, keep, &mut by_faces, "Sort loose photos into people folders by face", "Needs 'Keep my existing folders'.") {
                    self.set_option(|o| o.sort_by_faces = by_faces);
                }
                let mut rename = self.options.rename_files;
                if theme::toggle_row(ui, true, &mut rename, "Clean up file names", "Off: files keep their original names.") {
                    self.set_option(|o| o.rename_files = rename);
                }
            });
            ui.add_space(12.0);

            theme::card(ui, |ui| {
                ui.label(RichText::new("Music grouping").size(17.0).strong());
                ui.label(RichText::new("Words that mark a speaker in a file name, and names of AI music tools.").color(MUTED));
                ui.add_space(6.0);
                ui.label(RichText::new(rtl::for_display(&format!("Speaker words:  {}", self.options.speaker_keywords.join("، ")))).small());
                ui.label(RichText::new(rtl::for_display(&format!("AI tools:  {}", self.options.ai_markers.join(", ")))).small());
                ui.add_space(6.0);
                if ui.button("✏  Edit lists in Notepad").clicked() {
                    self.open_settings_file();
                }
            });
            ui.add_space(12.0);

            theme::card(ui, |ui| {
                ui.label(RichText::new("Data").size(17.0).strong());
                ui.label(RichText::new(rtl::for_display(&format!("Settings, undo record and face memory are kept in {}", self.storage.dir().display()))).small().color(MUTED));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Open data folder").clicked() {
                        let dir = self.storage.dir().to_path_buf();
                        let _ = std::fs::create_dir_all(&dir);
                        self.open_in_explorer(&dir);
                    }
                    let size = self.storage.face_cache_size();
                    if ui
                        .add_enabled(size > 0, egui::Button::new(format!("Clear face memory ({:.1} MB)", size as f64 / 1_048_576.0)))
                        .clicked()
                    {
                        self.confirm_clear_faces = true;
                    }
                });
            });
        });
    }

    fn about_page(&self, ui: &mut egui::Ui) {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            if let Some(logo) = &self.logo {
                ui.add(egui::Image::from_texture(egui::load::SizedTexture::new(logo.id(), [120.0, 120.0])));
            }
            ui.label(RichText::new("Array").size(32.0).strong());
            ui.label(RichText::new(format!("Version {}", VERSION)).color(MUTED));
            ui.add_space(12.0);
            ui.label("Gathers scattered files and puts them in tidy folders —");
            ui.label("photos into each person's folder, recitations and songs into each artist's folder —");
            ui.label("without touching what you already organized.");
            ui.add_space(16.0);
            ui.label(RichText::new("🔒 Everything runs on this computer. Nothing is uploaded.").color(GREEN));
            ui.add_space(16.0);
            ui.label(RichText::new("Face models: YuNet (MIT) and SFace (Apache 2.0) from OpenCV Zoo").small().color(MUTED));
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.confirm_undo {
            let count = self.undo_count.unwrap_or(0);
            egui::Window::new("Undo last organize")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("Move {} back to where they were before the last organize?", files(count)));
                    ui.label(RichText::new("Files that were changed or replaced since then are left alone.").small().color(MUTED));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if theme::primary_button(ui, true, "↩  Yes, undo").clicked() {
                            self.confirm_undo = false;
                            self.start_undo(ctx);
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_undo = false;
                        }
                    });
                });
        }
        if self.confirm_clear_faces {
            egui::Window::new("Clear face memory")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Photos will be analyzed again next time (a few minutes). Your photos are not affected.");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Clear").clicked() {
                            self.confirm_clear_faces = false;
                            self.status_message = match self.storage.clear_face_cache() {
                                Ok(()) => "Face memory cleared.".to_string(),
                                Err(err) => format!("⚠ {}", err),
                            };
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_clear_faces = false;
                        }
                    });
                });
        }
    }
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
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
            .frame(egui::Frame::none().fill(theme::SIDEBAR).inner_margin(egui::Margin::symmetric(16.0, 6.0)))
            .show(ctx, |ui| {
                let status = self.status_message.lines().next().unwrap_or_default();
                ui.add(egui::Label::new(RichText::new(rtl::for_display(status)).small().color(MUTED)).truncate(true));
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::BG).inner_margin(egui::Margin::symmetric(28.0, 20.0)))
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
