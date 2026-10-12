//! ترتيب الفيديو: سنة التصوير من الاسم أو من بيانات الملف نفسه، ومقاطع "الصور الحية"
//! (فيديو بنفس اسم صورة في نفس المجلد) التي تبقى بجانب صورتها.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use chrono::{DateTime, Datelike, Local};

use crate::naming;

/// الثواني بين 1904-01-01 (بداية التوقيت في ملفات MP4/MOV) و1970-01-01
const MP4_EPOCH_OFFSET: i64 = 2_082_844_800;

/// سنة تصوير الفيديو: من الاسم أولاً، ثم من تاريخ الإنشاء داخل الملف (MP4/MOV/M4V/3GP).
/// لا يُستخدم تاريخ تعديل الملف لأن النسخ من الجوال يغيّره إلى يوم النسخ.
pub fn year(path: &Path) -> Option<i32> {
    let stem = path.file_stem()?.to_string_lossy();
    naming::year_from_name(&stem).filter(|y| plausible(*y)).or_else(|| creation_year(path))
}

fn plausible(year: i32) -> bool {
    (1990..=Local::now().year() + 1).contains(&year)
}

fn creation_year(path: &Path) -> Option<i32> {
    creation_time(path).map(|t| t.year())
}

/// تاريخ التصوير من صندوق mvhd داخل moov (MP4/MOV/M4V/3GP)؛ None إن كان صفراً أو غير معقول
pub fn creation_time(path: &Path) -> Option<DateTime<Local>> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if !matches!(ext.as_str(), "mp4" | "mov" | "m4v" | "3gp") {
        return None;
    }
    let mut file = File::open(path).ok()?;
    let end = file.metadata().ok()?.len();
    let (moov_start, moov_end) = find_box(&mut file, 0, end, b"moov")?;
    let (mvhd_start, _) = find_box(&mut file, moov_start, moov_end, b"mvhd")?;
    file.seek(SeekFrom::Start(mvhd_start)).ok()?;
    let mut version = [0u8; 4];
    file.read_exact(&mut version).ok()?;
    let seconds = if version[0] == 1 {
        let mut b = [0u8; 8];
        file.read_exact(&mut b).ok()?;
        u64::from_be_bytes(b) as i64
    } else {
        let mut b = [0u8; 4];
        file.read_exact(&mut b).ok()?;
        u32::from_be_bytes(b) as i64
    };
    let local: DateTime<Local> = DateTime::from_timestamp(seconds - MP4_EPOCH_OFFSET, 0)?.into();
    Some(local).filter(|t| plausible(t.year()))
}

/// يبحث عن صندوق بنوع معيّن بين start وend، ويرجع بداية محتواه ونهايته
fn find_box(file: &mut File, mut pos: u64, end: u64, kind: &[u8; 4]) -> Option<(u64, u64)> {
    while pos + 8 <= end {
        file.seek(SeekFrom::Start(pos)).ok()?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header).ok()?;
        let mut size = u32::from_be_bytes(header[..4].try_into().ok()?) as u64;
        let mut content = pos + 8;
        if size == 1 {
            let mut large = [0u8; 8];
            file.read_exact(&mut large).ok()?;
            size = u64::from_be_bytes(large);
            content += 8;
        } else if size == 0 {
            size = end - pos;
        }
        if size < content - pos || pos + size > end {
            return None;
        }
        if &header[4..] == kind {
            return Some((content, pos + size));
        }
        pos += size;
    }
    None
}

/// مجلد السنة داخل مجلد الفيديو: مجلد موجود اسمه فيه هذه السنة وحدها ("فيديو 2019") وإلا "2019"
pub fn year_folder_name(year: i32, existing: &[String]) -> String {
    let wanted = year.to_string();
    existing
        .iter()
        .find(|name| name.trim() == wanted)
        .or_else(|| existing.iter().find(|name| years_in(name) == [year]))
        .cloned()
        .unwrap_or(wanted)
}

fn years_in(name: &str) -> Vec<i32> {
    let name = crate::normalizer::to_western_digits(name);
    let digits: Vec<char> = name.chars().collect();
    let mut years = Vec::new();
    let mut i = 0;
    while i < digits.len() {
        if digits[i].is_ascii_digit() {
            let start = i;
            while i < digits.len() && digits[i].is_ascii_digit() {
                i += 1;
            }
            if i - start == 4 {
                if let Ok(y) = digits[start..i].iter().collect::<String>().parse::<i32>() {
                    if (1900..2100).contains(&y) {
                        years.push(y);
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    years
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ملف MP4 صغير فيه ftyp ثم moov/mvhd بتاريخ إنشاء معيّن
    fn fake_mp4(path: &Path, unix_seconds: i64, version: u8) {
        let mut mvhd = vec![version, 0, 0, 0];
        let t = unix_seconds + MP4_EPOCH_OFFSET;
        if version == 1 {
            mvhd.extend((t as u64).to_be_bytes());
        } else {
            mvhd.extend((t as u32).to_be_bytes());
        }
        mvhd.extend([0u8; 20]);
        let boxed = |kind: &[u8], body: &[u8]| [((body.len() + 8) as u32).to_be_bytes().as_slice(), kind, body].concat();
        let data = [boxed(b"ftyp", b"qt  \0\0\0\0"), boxed(b"free", &[0u8; 5]), boxed(b"moov", &boxed(b"mvhd", &mvhd))].concat();
        std::fs::write(path, data).unwrap();
    }

    #[test]
    fn year_comes_from_the_name_first() {
        assert_eq!(year(Path::new("20260219_183536_IMG_2755.MOV")), Some(2026));
        assert_eq!(year(Path::new("VID-20190704-WA0003.mp4")), Some(2019));
    }

    #[test]
    fn year_comes_from_the_video_metadata() {
        let dir = std::env::temp_dir().join("array_video_meta");
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("IMG_0548.MOV");
        fake_mp4(&a, 1_563_000_000, 0); // 2019-07-13
        assert_eq!(year(&a), Some(2019));
        let b = dir.join("clip.mp4");
        fake_mp4(&b, 1_700_000_000, 1); // 2023-11-14
        assert_eq!(year(&b), Some(2023));
        // تاريخ صفري (1904) يعني أن الكاميرا لم تكتب تاريخاً
        let c = dir.join("zero.mp4");
        fake_mp4(&c, -MP4_EPOCH_OFFSET, 0);
        assert_eq!(year(&c), None);
        // ملف تالف أو ليس MP4
        let d = dir.join("broken.mov");
        std::fs::write(&d, b"not a video").unwrap();
        assert_eq!(year(&d), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn existing_year_folders_are_reused() {
        let existing = vec!["فيديو 2019".to_string(), "2020".to_string(), "رحلة 2018 و2019".to_string()];
        assert_eq!(year_folder_name(2019, &existing), "فيديو 2019");
        assert_eq!(year_folder_name(2020, &existing), "2020");
        assert_eq!(year_folder_name(2021, &existing), "2021");
        assert_eq!(year_folder_name(2018, &existing), "2018");
    }
}
