pub fn normalize_arabic(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'أ' | 'إ' | 'آ' => 'ا',
            'ة' => 'ه',
            'ى' => 'ي',
            _ => c,
        })
        .collect()
}

pub fn sanitize_filename(name: &str) -> String {
    let invalid_chars = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let cleaned: String = name
        .chars()
        .filter(|c| !invalid_chars.contains(c))
        .collect();

    let trimmed = cleaned.trim().to_string();
    if trimmed.is_empty() {
        "Unknown".to_string()
    } else {
        trimmed
    }
}