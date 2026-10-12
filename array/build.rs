use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=assets/app.ico");
    if cfg!(target_os = "windows") {
        let icon_path = "assets/app.ico";
        if Path::new(icon_path).exists() {
            let mut res = winres::WindowsResource::new();
            res.set_icon(icon_path);
            // معلومات البرنامج كما تظهر في خصائص الملف وفي مدير المهام
            res.set("ProductName", "Array");
            res.set("FileDescription", "Array - organizes your files");
            res.set("OriginalFilename", "Array.exe");
            res.set("LegalCopyright", "Array");
            if let Err(e) = res.compile() {
                eprintln!("Warning: Failed to compile icon resource: {}", e);
            }
        } else {
            eprintln!("Warning: Icon file '{}' not found. Building without icon.", icon_path);
        }
    }
}
