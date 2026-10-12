use std::path::Path;

fn main() {
    if cfg!(target_os = "windows") {
        let icon_path = "Array.ico";
        if Path::new(icon_path).exists() {
            let mut res = winres::WindowsResource::new();
            res.set_icon(icon_path);
            if let Err(e) = res.compile() {
                eprintln!("Warning: Failed to compile icon resource: {}", e);
            }
        } else {
            eprintln!("Warning: Icon file '{}' not found. Building without icon.", icon_path);
        }
    }
}