use std::path::Path;

pub struct SafetyChecker;

impl SafetyChecker {
    pub fn is_safe_to_process(path: &Path) -> bool {
        if let Some(path_str) = path.to_str() {
            let lower = path_str.to_lowercase();
            if lower.contains("system32") 
                || lower.contains("windows") 
                || lower.contains("program files") 
                || lower.ends_with("c:") 
                || lower.ends_with("c:\\") 
            {
                return false;
            }
        }
        true
    }
}