#![windows_subsystem = "windows"]

mod grouping;
mod naming;
mod normalizer;
mod organizer;
mod rtl;
mod safety;
mod storage;

use eframe::egui;
use organizer::{Options, OrganizerEngine, Plan};
use rfd::FileDialog;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use storage::{Storage, UndoLog};

fn main() -> eframe::Result<()> {
    let icon = load_app_icon();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 560.0])
            .with_min_inner_size([620.0, 460.0])
            .with_icon(Arc::new(icon)),
        ..Default::default()
    };

    eframe::run_native(
        "Array File Organizer v1.0",
        options,
        Box::new(|cc| {
            rtl::install_arabic_font(&cc.egui_ctx);
            Box::new(ArrayApp::new(Storage::default_location()))
        }),
    )
}

/// أيقونة النافذة وشريط المهام من نفس ملف Array.ico المضمّن في ملف EXE
fn load_app_icon() -> egui::IconData {
    match image::load_from_memory_with_format(include_bytes!("../Array.ico"), image::ImageFormat::Ico) {
        Ok(img) => {
            let rgba = img.into_rgba8();
            let (width, height) = rgba.dimensions();
            egui::IconData { rgba: rgba.into_raw(), width, height }
        }
        Err(_) => egui::IconData::default(),
    }
}

/// عمل جارٍ في الخلفية حتى لا تتجمد النافذة مع المجلدات الكبيرة
enum Job {
    Planning(Receiver<Result<Plan, String>>),
    /// تنظيم أو تراجع، مع عدّاد الملفات المنجزة لشريط التقدم
    Moving {
        label: &'static str,
        rx: Receiver<Result<String, String>>,
        progress: Arc<AtomicUsize>,
        total: usize,
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
    headline: String,
    summary: String,
    rows: Vec<(String, String)>,
    /// مجلدات برامج ومشاريع تُركت كما هي، مع السبب
    skipped: Vec<String>,
}

impl Preview {
    fn new(plan: Plan, rename_files: bool) -> Self {
        let relative = |path: &Path, base: &Path| path.strip_prefix(base).unwrap_or(path).display().to_string();

        // عدد الملفات لكل مجلد فئة، وعدد مجلدات الفنانين داخل مجلد الموسيقى
        let mut per_category: BTreeMap<String, usize> = BTreeMap::new();
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
            // ملف يبقى في المجلد المختار نفسه (يُعاد تسميته فقط) يُحسب باسم "This folder"
            let category = if parts.len() > 1 { parts.first().cloned() } else { Some(THIS_FOLDER.to_string()) };
            if let Some(category) = category.as_ref() {
                *per_category.entry(category.clone()).or_default() += 1;
                // مجلد فرعي داخل مجلد الفئة (يحدث فقط للموسيقى) = مجلد فنان
                if parts.len() > 2 && parts[1] != "Single Tracks" {
                    artist_folders.entry(category.clone()).or_default().insert(parts[1].clone());
                }
            }
        }
        let summary = per_category
            .iter()
            .map(|(category, n)| {
                if category == THIS_FOLDER {
                    return format!("{}: {}", category, n);
                }
                let state = if plan.new_dirs.contains(&plan.source.join(category)) { "new" } else { "existing" };
                match artist_folders.get(category).map_or(0, |s| s.len()) {
                    0 => format!("{} ({} folder): {}", category, state, n),
                    folders => format!(
                        "{} ({} folder): {}, {} artist folder{}",
                        category, state, n, folders, if folders == 1 { "" } else { "s" }
                    ),
                }
            })
            .collect::<Vec<_>>()
            .join("   ·   ");

        let mut headline = format!(
            "{} will be moved{}.",
            files(plan.moves.len()),
            if rename_files { " or renamed" } else { "" }
        );
        if plan.in_place > 0 {
            headline.push_str(&format!(" {} already in the right place.", files(plan.in_place)));
        }
        if plan.kept_in_folders > 0 {
            headline.push_str(&format!(" {} in your existing folders are left as they are.", files(plan.kept_in_folders)));
        }
        let rows = plan
            .moves
            .iter()
            .map(|m| (rtl::for_display(&relative(&m.from, &plan.source)), rtl::for_display(&relative(&m.to, &plan.source))))
            .collect();

        let skipped = plan
            .skipped
            .iter()
            .map(|(dir, reason)| rtl::for_display(&format!("{}  —  {}", relative(dir, &plan.source), reason)))
            .collect();

        Self { plan, headline, summary, rows, skipped }
    }
}

struct ArrayApp {
    storage: Storage,
    options: Options,
    selected_path: Option<PathBuf>,
    status_message: String,
    job: Option<Job>,
    preview: Option<Preview>,
    /// عدد الملفات التي يمكن إرجاعها (None إذا لا يوجد ما يُتراجع عنه)
    undo_count: Option<usize>,
    /// عدد السجلات في ملف التراجع (لشريط التقدم)
    undo_entries: usize,
    confirm_undo: bool,
}

impl ArrayApp {
    fn new(storage: Storage) -> Self {
        let (options, status_message) = match storage.load_settings() {
            Ok(options) => (options, "Please select a folder to organize...".to_string()),
            Err(err) => (Options::default(), format!("⚠ {}", err)),
        };
        let mut app = Self {
            storage,
            options,
            selected_path: None,
            status_message,
            job: None,
            preview: None,
            undo_count: None,
            undo_entries: 0,
            confirm_undo: false,
        };
        app.refresh_undo();
        app
    }

    fn refresh_undo(&mut self) {
        // بعد تنظيم توقف في المنتصف يحتوي السجل ملفات لم تُنقل، فلا تُحسب
        let (entries, movable) = match self.storage.load_undo() {
            Ok(Some(log)) => (log.moves.len(), log.moves.iter().filter(|m| m.to.exists()).count()),
            Ok(None) => (0, 0),
            Err(err) => {
                self.status_message = format!("⚠ {}", err);
                (0, 0)
            }
        };
        self.undo_entries = entries;
        self.undo_count = Some(movable).filter(|&n| n > 0);
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
                    self.status_message = if plan.kept_in_folders > 0 {
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
                        self.status_message.push_str(&format!(
                            " {} left untouched.",
                            folders(plan.skipped.len())
                        ));
                    }
                }
                Some(Ok(plan)) => {
                    self.status_message = "Review where each file will go, then confirm.".to_string();
                    self.preview = Some(Preview::new(plan, self.options.rename_files));
                }
                Some(Err(err)) => self.status_message = format!("❌ Error: {}", err),
            },
            Job::Moving { rx, .. } => match poll(rx) {
                None => return,
                Some(Ok(msg)) => self.status_message = msg,
                Some(Err(err)) => self.status_message = format!("❌ Error: {}", err),
            },
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

    fn start_planning(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.reload_settings();
        let options = self.options.clone();
        self.job = Some(Job::Planning(spawn_job(ctx, move || OrganizerEngine::plan(&path, &options))));
        self.status_message = "⏳ Scanning files...".to_string();
    }

    fn start_organizing(&mut self, ctx: &egui::Context, plan: Plan) {
        let progress = Arc::new(AtomicUsize::new(0));
        let total = plan.moves.len();
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

            let mut msg = format!("✔ Organized {} files in {}", result.done.len(), plan.source.display());
            if !result.failed.is_empty() {
                msg.push_str(&format!(
                    "\n⚠ {} files could not be moved (in use, access denied, or a file with the same name appeared) and were left in place.",
                    result.failed.len()
                ));
            }
            if let Err(e) = saved {
                msg.push_str(&format!("\n⚠ {}", e));
            }
            Ok(msg)
        });
        self.job = Some(Job::Moving { label: "Organizing", rx, progress, total });
        self.status_message = "⏳ Organizing... please wait".to_string();
    }

    fn start_undo(&mut self, ctx: &egui::Context) {
        let progress = Arc::new(AtomicUsize::new(0));
        let total = self.undo_entries;
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

            let mut msg = format!("↩ Restored {} files to their original places in {}", result.done.len(), log.source.display());
            if !result.failed.is_empty() {
                msg.push_str(&format!(
                    "\n⚠ {} files could not be restored (moved, renamed, in use, or a new file has the same name). Press Undo again to retry them.",
                    result.failed.len()
                ));
            }
            Ok(msg)
        });
        self.job = Some(Job::Moving { label: "Restoring", rx, progress, total });
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
            Ok(_) => "Edit \"speaker_keywords\" in Notepad and save. The new words are used from the next preview.".to_string(),
            Err(err) => format!("⚠ Could not open {}: {}", path.display(), err),
        };
    }

    fn show_controls(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let busy = self.busy();

        ui.vertical_centered(|ui| {
            ui.add_space(12.0);
            ui.heading("🚀 Array File Organizer");
            ui.label("Smart Media & Document Organization Engine");
            ui.add_space(14.0);

            if ui.add_enabled(!busy, egui::Button::new("📁 Select Folder")).clicked() {
                if let Some(path) = FileDialog::new().pick_folder() {
                    self.selected_path = Some(path);
                    self.status_message = "Ready. Press Preview to see where each file will go.".to_string();
                }
            }
            if let Some(path) = &self.selected_path {
                ui.add_space(4.0);
                ui.label(rtl::for_display(&format!("Selected: {}", path.display())));
            }
        });

        ui.add_space(10.0);
        ui.columns(2, |cols| {
            cols[0].with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                let mut rename = self.options.rename_files;
                if ui
                    .add_enabled(!busy, egui::Checkbox::new(&mut rename, "Clean up file names"))
                    .on_hover_text("Off: files keep their original names and are only sorted into folders.")
                    .changed()
                {
                    self.set_option(|o| o.rename_files = rename);
                }
            });
            cols[1].with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                if ui
                    .add_enabled(!busy, egui::Button::new("⚙ Edit speaker keywords"))
                    .on_hover_text("Words that mark the speaker at the end of a file name, e.g. \"Lecture - Sheikh X\".")
                    .clicked()
                {
                    self.open_settings_file();
                }
            });
        });

        ui.add_space(4.0);
        ui.vertical_centered(|ui| {
            let mut keep = self.options.keep_existing_folders;
            if ui
                .add_enabled(!busy, egui::Checkbox::new(&mut keep, "Keep my existing folders as they are"))
                .on_hover_text(
                    "On: files already inside subfolders are not moved or renamed; only loose files are organized \
                     (they can still go into a matching existing folder).\nOff: all subfolders are emptied and rebuilt.",
                )
                .changed()
            {
                self.set_option(|o| o.keep_existing_folders = keep);
            }
        });

        ui.add_space(10.0);
        ui.columns(2, |cols| {
            cols[0].with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                let can_start = !busy && self.selected_path.is_some();
                if ui.add_enabled(can_start, egui::Button::new("🔍 Preview & Organize")).clicked() {
                    if let Some(path) = self.selected_path.clone() {
                        self.start_planning(ctx, path);
                    }
                }
            });
            cols[1].with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                let label = match self.undo_count {
                    Some(n) => format!("↩ Undo last organize ({} files)", n),
                    None => "↩ Undo last organize".to_string(),
                };
                if ui.add_enabled(!busy && self.undo_count.is_some(), egui::Button::new(label)).clicked() {
                    self.confirm_undo = true;
                }
            });
        });
    }

    fn show_progress(&self, ui: &mut egui::Ui) {
        match &self.job {
            Some(Job::Planning(_)) => {
                ui.add_space(16.0);
                ui.vertical_centered(|ui| {
                    ui.horizontal(|ui| {
                        ui.add_space((ui.available_width() - 160.0).max(0.0) / 2.0);
                        ui.spinner();
                        ui.label("Scanning files...");
                    });
                });
            }
            Some(Job::Moving { label, progress, total, .. }) => {
                let done = progress.load(Ordering::Relaxed);
                let fraction = if *total == 0 { 1.0 } else { done as f32 / *total as f32 };
                ui.add_space(16.0);
                ui.add(egui::ProgressBar::new(fraction).text(format!("{}: {} / {} files", label, done, total)));
            }
            None => {}
        }
    }

    fn show_preview(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(preview) = &self.preview else { return };
        let mut decision: Option<bool> = None;

        ui.add_space(10.0);
        ui.separator();
        ui.label(egui::RichText::new(&preview.headline).strong());
        ui.label(rtl::for_display(&preview.summary));
        if !preview.skipped.is_empty() {
            egui::CollapsingHeader::new(format!(
                "🛡 {} will not be touched (click to see)",
                folders(preview.skipped.len())
            ))
            .id_source("skipped_folders")
            .show(ui, |ui| {
                egui::ScrollArea::vertical().id_source("skipped_list").max_height(90.0).show(ui, |ui| {
                    for line in &preview.skipped {
                        ui.label(line);
                    }
                });
            });
        }
        ui.add_space(4.0);

        let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
        let table_height = (ui.available_height() - 44.0).max(row_height * 3.0);
        egui::ScrollArea::vertical()
            .max_height(table_height)
            .auto_shrink([false, false])
            .show_rows(ui, row_height, preview.rows.len(), |ui, range| {
                let column = ((ui.available_width() - 30.0) / 2.0).max(50.0);
                for (from, to) in &preview.rows[range] {
                    ui.horizontal(|ui| {
                        table_cell(ui, column, row_height, from);
                        ui.label("→");
                        table_cell(ui, column, row_height, to);
                    });
                }
            });

        ui.add_space(6.0);
        ui.columns(2, |cols| {
            cols[0].with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if ui.button(format!("✔ Organize {} files", preview.rows.len())).clicked() {
                    decision = Some(true);
                }
            });
            cols[1].with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                if ui.button("✖ Cancel").clicked() {
                    decision = Some(false);
                }
            });
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

    fn show_undo_confirmation(&mut self, ctx: &egui::Context) {
        if !self.confirm_undo {
            return;
        }
        let count = self.undo_count.unwrap_or(0);
        egui::Window::new("Undo")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("Move {} files back to where they were before the last organize?", count));
                ui.label("Files that were changed or replaced since then are left alone.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("↩ Yes, undo").clicked() {
                        self.confirm_undo = false;
                        self.start_undo(ctx);
                    }
                    if ui.button("✖ Cancel").clicked() {
                        self.confirm_undo = false;
                    }
                });
            });
    }
}

fn files(n: usize) -> String {
    if n == 1 { "1 file".to_string() } else { format!("{} files", n) }
}

fn folders(n: usize) -> String {
    if n == 1 { "1 program or project folder".to_string() } else { format!("{} program or project folders", n) }
}

/// خلية بعرض ثابت؛ الاسم الطويل يُختصر ويظهر كاملاً عند المرور عليه
fn table_cell(ui: &mut egui::Ui, width: f32, height: f32, text: &str) {
    ui.allocate_ui_with_layout(egui::vec2(width, height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(width);
        ui.add(egui::Label::new(text).truncate(true)).on_hover_text(text);
    });
}

impl eframe::App for ArrayApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_job();
        if self.job.is_some() {
            // تحديث شريط التقدم أثناء العمل
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.vertical_centered(|ui| ui.label(rtl::for_display(&self.status_message)));
            ui.add_space(6.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            self.show_controls(ui, ctx);
            self.show_progress(ui);
            self.show_preview(ui, ctx);
        });

        self.show_undo_confirmation(ctx);
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
}
