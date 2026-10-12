#![windows_subsystem = "windows"]

mod normalizer;
mod organizer;
mod rtl;
mod safety;

use eframe::egui;
use organizer::OrganizerEngine;
use rfd::FileDialog;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;

fn main() -> eframe::Result<()> {
    let icon = create_app_icon();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([540.0, 320.0])
            .with_resizable(false)
            .with_icon(Arc::new(icon)),
        ..Default::default()
    };

    eframe::run_native(
        "Array File Organizer v1.0",
        options,
        Box::new(|cc| {
            rtl::install_arabic_font(&cc.egui_ctx);
            Box::new(ArrayApp::default())
        }),
    )
}

/// رسم أيقونة عالي الدقة (64x64) لمجلد أنيق لضمان الظهور في شريط المهام
fn create_app_icon() -> egui::IconData {
    let size = 64;
    let mut rgba = vec![0u8; size * size * 4];

    for y in 0..size {
        for x in 0..size {
            let idx = (y * size + x) * 4;
            let dx = x as f32 - 31.5;
            let dy = y as f32 - 31.5;
            let dist_sq = dx * dx + dy * dy;

            // خلفية دائرة دائرية بحواف متناسقة
            if dist_sq <= 29.0 * 29.0 {
                // رسم مجلد برتقالي واصل في المنتصف
                if y >= 20 && y <= 46 && x >= 14 && x <= 50 {
                    if y <= 25 && x > 30 {
                        // الخلفية خلف لسان المجلد
                        rgba[idx] = 0x15;
                        rgba[idx + 1] = 0x18;
                        rgba[idx + 2] = 0x21;
                        rgba[idx + 3] = 0xFF;
                    } else {
                        // ألوان المجلد البرتقالي الذهبي
                        rgba[idx] = 0xFF;
                        rgba[idx + 1] = 0x98;
                        rgba[idx + 2] = 0x00;
                        rgba[idx + 3] = 0xFF;
                    }
                } else {
                    // لون خلفية الدائرة الكحلي
                    rgba[idx] = 0x15;
                    rgba[idx + 1] = 0x18;
                    rgba[idx + 2] = 0x21;
                    rgba[idx + 3] = 0xFF;
                }
            }
        }
    }

    egui::IconData {
        rgba,
        width: size as u32,
        height: size as u32,
    }
}

/// عمل جارٍ في الخلفية حتى لا تتجمد النافذة مع المجلدات الكبيرة
enum Job {
    Counting(Receiver<Result<usize, String>>),
    Organizing(Receiver<Result<String, String>>),
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
        Err(TryRecvError::Disconnected) => Some(Err("organizing stopped unexpectedly".to_string())),
        Err(TryRecvError::Empty) => None,
    }
}

struct ArrayApp {
    selected_path: Option<PathBuf>,
    status_message: String,
    job: Option<Job>,
    /// عدد الملفات بانتظار تأكيد المستخدم قبل النقل
    confirm_count: Option<usize>,
}

impl Default for ArrayApp {
    fn default() -> Self {
        Self {
            selected_path: None,
            status_message: "Please select a folder to organize...".to_string(),
            job: None,
            confirm_count: None,
        }
    }
}

impl ArrayApp {
    fn poll_job(&mut self) {
        let finished = match &self.job {
            Some(Job::Counting(rx)) => poll(rx).map(|result| match result {
                Ok(0) => "No files to organize in this folder.".to_string(),
                Ok(n) => {
                    self.confirm_count = Some(n);
                    "Waiting for confirmation...".to_string()
                }
                Err(err) => format!("❌ Error: {}", err),
            }),
            Some(Job::Organizing(rx)) => poll(rx).map(|result| match result {
                Ok(msg) => msg,
                Err(err) => format!("❌ Error: {}", err),
            }),
            None => None,
        };
        if let Some(msg) = finished {
            self.status_message = msg;
            self.job = None;
        }
    }

    fn show_confirmation(&mut self, ctx: &egui::Context) {
        let (Some(count), Some(path)) = (self.confirm_count, self.selected_path.clone()) else {
            return;
        };
        egui::Window::new("Confirm")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("{} files will be moved and renamed into \"Array_Organized\".", count));
                ui.label("This cannot be undone. Continue?");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("✔ Yes, organize").clicked() {
                        self.confirm_count = None;
                        self.job = Some(Job::Organizing(spawn_job(ctx, move || OrganizerEngine::process_directory(&path))));
                        self.status_message = "⏳ Organizing... please wait".to_string();
                    }
                    if ui.button("✖ Cancel").clicked() {
                        self.confirm_count = None;
                        self.status_message = "Cancelled. No files were moved.".to_string();
                    }
                });
            });
    }
}

impl eframe::App for ArrayApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_job();
        let busy = self.job.is_some() || self.confirm_count.is_some();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.heading("🚀 Array File Organizer");
                ui.label("Smart Media & Document Organization Engine");
                ui.add_space(20.0);

                if ui.add_enabled(!busy, egui::Button::new("📁 Select Folder")).clicked() {
                    if let Some(path) = FileDialog::new().pick_folder() {
                        self.selected_path = Some(path);
                        self.status_message = "Ready to start organizing...".to_string();
                    }
                }

                ui.add_space(10.0);

                if let Some(ref path) = self.selected_path {
                    ui.label(rtl::for_display(&format!("Selected: {}", path.display())));
                    ui.add_space(15.0);

                    if ui.add_enabled(!busy, egui::Button::new("⚡ Start Organizing Now")).clicked() {
                        // عدّ الملفات أولاً ثم طلب التأكيد قبل أي نقل
                        let path = path.clone();
                        self.job = Some(Job::Counting(spawn_job(ctx, move || OrganizerEngine::count_files(&path))));
                        self.status_message = "⏳ Counting files...".to_string();
                    }
                }

                ui.add_space(20.0);
                ui.separator();
                ui.add_space(10.0);
                ui.label(rtl::for_display(&self.status_message));
            });
        });

        self.show_confirmation(ctx);
    }
}
