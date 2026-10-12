#![windows_subsystem = "windows"]

mod normalizer;
mod organizer;
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
        Box::new(|_cc| Box::new(ArrayApp::default())),
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

struct ArrayApp {
    selected_path: Option<PathBuf>,
    status_message: String,
    /// نتيجة التنظيم الجاري في الخلفية (None عندما لا يوجد عمل)
    pending: Option<Receiver<Result<String, String>>>,
}

impl Default for ArrayApp {
    fn default() -> Self {
        Self {
            selected_path: None,
            status_message: "Please select a folder to organize...".to_string(),
            pending: None,
        }
    }
}

impl eframe::App for ArrayApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(rx) = &self.pending {
            let finished = match rx.try_recv() {
                Ok(Ok(msg)) => Some(msg),
                Ok(Err(err)) => Some(format!("❌ Error: {}", err)),
                Err(TryRecvError::Disconnected) => Some("❌ Error: organizing stopped unexpectedly".to_string()),
                Err(TryRecvError::Empty) => None,
            };
            if let Some(msg) = finished {
                self.status_message = msg;
                self.pending = None;
            }
        }
        let busy = self.pending.is_some();

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
                    ui.label(format!("Selected: {}", path.display()));
                    ui.add_space(15.0);

                    if ui.add_enabled(!busy, egui::Button::new("⚡ Start Organizing Now")).clicked() {
                        // العمل في خيط منفصل حتى لا تتجمد النافذة مع المجلدات الكبيرة
                        let (tx, rx) = mpsc::channel();
                        let path = path.clone();
                        let ctx = ctx.clone();
                        thread::spawn(move || {
                            let _ = tx.send(OrganizerEngine::process_directory(&path));
                            ctx.request_repaint();
                        });
                        self.pending = Some(rx);
                        self.status_message = "⏳ Organizing... please wait".to_string();
                    }
                }

                ui.add_space(20.0);
                ui.separator();
                ui.add_space(10.0);
                ui.label(&self.status_message);
            });
        });
    }
}