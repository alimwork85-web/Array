//! التعرّف على الوجوه على الجهاز نفسه (بدون إنترنت) لترتيب الصور المتفرقة في مجلدات الأشخاص.
//!
//! النموذجان من OpenCV Zoo ومضمّنان في البرنامج:
//! - YuNet (MIT): يكتشف الوجوه ويعطي 5 نقاط (العينان، الأنف، طرفا الفم).
//! - SFace (Apache 2.0): يحوّل الوجه بعد محاذاته إلى بصمة من 128 رقماً تُقارن بجيب التمام.

use std::collections::HashMap;
use std::fs;
use std::io::{BufReader, Cursor};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::UNIX_EPOCH;

use image::imageops::FilterType;
use image::{DynamicImage, RgbImage};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use tract_onnx::prelude::*;

const DETECTOR_ONNX: &[u8] = include_bytes!("../models/face_detection_yunet_2023mar.onnx");
const RECOGNIZER_ONNX: &[u8] = include_bytes!("../models/face_recognition_sface_2021dec.onnx");

const DET_SIZE: usize = 640;
/// أكبر بُعد للصورة قبل التحليل (أسرع، ويكفي لوجوه واضحة)
const ANALYSIS_MAX: u32 = 1280;
const DET_SCORE: f32 = 0.85;
const NMS_IOU: f32 = 0.3;
/// وجه أصغر من هذا (بالبكسل في صورة التحليل) لا يُعتمد عليه في التعرّف
const MIN_FACE: f32 = 40.0;

/// مواضع النقاط الخمس في الوجه المحاذى 112×112 (قالب SFace/ArcFace)
const TEMPLATE: [(f32, f32); 5] = [(38.2946, 51.6963), (73.5318, 51.5014), (56.0252, 71.7366), (41.5493, 92.3655), (70.7299, 92.2041)];

type Model = TypedRunnableModel<TypedModel>;

pub struct FaceEngine {
    detector: Model,
    recognizer: Model,
}

/// يُحمَّل مرة واحدة فقط عند أول حاجة
static ENGINE: LazyLock<Result<FaceEngine, String>> = LazyLock::new(FaceEngine::load);

pub fn engine() -> Result<&'static FaceEngine, String> {
    ENGINE.as_ref().map_err(|e| e.clone())
}

#[derive(Debug, Clone)]
struct Detection {
    score: f32,
    bbox: [f32; 4],
    landmarks: [(f32, f32); 5],
}

/// بصمة وجه مطبَّعة (طولها 1)
pub type Embedding = Vec<f32>;

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn load_model(bytes: &[u8], shape: [usize; 4]) -> Result<Model, String> {
    tract_onnx::onnx()
        .model_for_read(&mut Cursor::new(bytes))
        .and_then(|m| m.with_input_fact(0, f32::fact(shape).into()))
        .and_then(|m| m.into_optimized())
        .and_then(|m| m.into_runnable())
        .map_err(|e| format!("{} {e}", crate::i18n::tr("Could not load the face model:", "تعذّر تحميل نموذج الوجوه:")))
}

/// قراءة الصورة مع تطبيق اتجاه EXIF (صور الجوال تُحفظ مائلة ويُكتب اتجاهها في EXIF)
pub fn load_image(path: &Path) -> Option<RgbImage> {
    let img = image::open(path).ok()?;
    let orientation = fs::File::open(path)
        .ok()
        .and_then(|f| exif::Reader::new().read_from_container(&mut BufReader::new(f)).ok())
        .and_then(|e| e.get_field(exif::Tag::Orientation, exif::In::PRIMARY).and_then(|f| f.value.get_uint(0)))
        .unwrap_or(1);
    let img = match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    };
    let img = if img.width().max(img.height()) > ANALYSIS_MAX {
        img.resize(ANALYSIS_MAX, ANALYSIS_MAX, FilterType::Triangle)
    } else {
        img
    };
    Some(DynamicImage::into_rgb8(img))
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let (x1, y1) = (a[0].max(b[0]), a[1].max(b[1]));
    let (x2, y2) = ((a[0] + a[2]).min(b[0] + b[2]), (a[1] + a[3]).min(b[1] + b[3]));
    let inter = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    inter / (a[2] * a[3] + b[2] * b[3] - inter).max(1e-6)
}

impl FaceEngine {
    fn load() -> Result<Self, String> {
        Ok(Self {
            detector: load_model(DETECTOR_ONNX, [1, 3, DET_SIZE, DET_SIZE])?,
            recognizer: load_model(RECOGNIZER_ONNX, [1, 3, 112, 112])?,
        })
    }

    fn detect(&self, img: &RgbImage) -> Vec<Detection> {
        // تصغير مع الحفاظ على النسبة ثم حشو إلى 640×640؛ YuNet يأخذ BGR بقيم 0..255
        let scale = DET_SIZE as f32 / img.width().max(img.height()) as f32;
        let (w, h) = (((img.width() as f32 * scale).round() as u32).max(1), ((img.height() as f32 * scale).round() as u32).max(1));
        let small = image::imageops::resize(img, w, h, FilterType::Triangle);
        let mut input = Tensor::zero::<f32>(&[1, 3, DET_SIZE, DET_SIZE]).unwrap();
        {
            let mut view = input.to_array_view_mut::<f32>().unwrap();
            for (x, y, p) in small.enumerate_pixels() {
                let (x, y) = (x as usize, y as usize);
                view[[0, 0, y, x]] = p[2] as f32;
                view[[0, 1, y, x]] = p[1] as f32;
                view[[0, 2, y, x]] = p[0] as f32;
            }
        }
        let Ok(out) = self.detector.run(tvec!(input.into())) else { return Vec::new() };
        let slice = |i: usize| out[i].as_slice::<f32>().unwrap_or(&[]);

        let mut found = Vec::new();
        for (k, stride) in [8usize, 16, 32].into_iter().enumerate() {
            let cols = DET_SIZE / stride;
            let (cls, obj, bbox, kps) = (slice(k), slice(k + 3), slice(k + 6), slice(k + 9));
            for idx in 0..cls.len() {
                let score = (cls[idx].clamp(0.0, 1.0) * obj[idx].clamp(0.0, 1.0)).sqrt();
                if score < DET_SCORE {
                    continue;
                }
                let (r, c) = ((idx / cols) as f32, (idx % cols) as f32);
                let s = stride as f32;
                let cx = (c + bbox[idx * 4]) * s;
                let cy = (r + bbox[idx * 4 + 1]) * s;
                let bw = bbox[idx * 4 + 2].exp() * s;
                let bh = bbox[idx * 4 + 3].exp() * s;
                let mut landmarks = [(0.0, 0.0); 5];
                for (n, lm) in landmarks.iter_mut().enumerate() {
                    *lm = ((kps[idx * 10 + 2 * n] + c) * s / scale, (kps[idx * 10 + 2 * n + 1] + r) * s / scale);
                }
                found.push(Detection {
                    score,
                    bbox: [(cx - bw / 2.0) / scale, (cy - bh / 2.0) / scale, bw / scale, bh / scale],
                    landmarks,
                });
            }
        }
        // حذف المكرر (NMS)
        found.sort_by(|a, b| b.score.total_cmp(&a.score));
        let mut kept: Vec<Detection> = Vec::new();
        for d in found {
            if kept.iter().all(|k| iou(&k.bbox, &d.bbox) < NMS_IOU) {
                kept.push(d);
            }
        }
        kept
    }

    /// محاذاة الوجه إلى 112×112 بتحويل تشابه (دوران + تكبير + إزاحة) يطابق النقاط الخمس مع القالب
    fn align(img: &RgbImage, landmarks: &[(f32, f32); 5]) -> RgbImage {
        let n = 5.0;
        let (mpx, mpy) = (landmarks.iter().map(|p| p.0).sum::<f32>() / n, landmarks.iter().map(|p| p.1).sum::<f32>() / n);
        let (mqx, mqy) = (TEMPLATE.iter().map(|p| p.0).sum::<f32>() / n, TEMPLATE.iter().map(|p| p.1).sum::<f32>() / n);
        let (mut num_a, mut num_b, mut den) = (0.0, 0.0, 0.0);
        for (p, q) in landmarks.iter().zip(TEMPLATE.iter()) {
            let (ax, ay, bx, by) = (p.0 - mpx, p.1 - mpy, q.0 - mqx, q.1 - mqy);
            num_a += ax * bx + ay * by;
            num_b += ax * by - ay * bx;
            den += ax * ax + ay * ay;
        }
        let (a, b) = (num_a / den.max(1e-6), num_b / den.max(1e-6));
        let (tx, ty) = (mqx - (a * mpx - b * mpy), mqy - (b * mpx + a * mpy));
        let det = (a * a + b * b).max(1e-9);

        let mut out = RgbImage::new(112, 112);
        for (u, v, px) in out.enumerate_pixels_mut() {
            // من نقطة في الوجه المحاذى إلى مكانها في الصورة الأصلية
            let (qx, qy) = (u as f32 - tx, v as f32 - ty);
            let x = (a * qx + b * qy) / det;
            let y = (-b * qx + a * qy) / det;
            *px = bilinear(img, x, y);
        }
        out
    }

    fn embed(&self, face: &RgbImage) -> Option<Embedding> {
        // SFace يأخذ RGB بقيم 0..255
        let mut input = Tensor::zero::<f32>(&[1, 3, 112, 112]).unwrap();
        {
            let mut view = input.to_array_view_mut::<f32>().unwrap();
            for (x, y, p) in face.enumerate_pixels() {
                let (x, y) = (x as usize, y as usize);
                for ch in 0..3 {
                    view[[0, ch, y, x]] = p[ch] as f32;
                }
            }
        }
        let out = self.recognizer.run(tvec!(input.into())).ok()?;
        let v: Vec<f32> = out[0].as_slice::<f32>().ok()?.to_vec();
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
        Some(v.into_iter().map(|x| x / norm).collect())
    }

    /// بصمات الوجوه الواضحة في الصورة. None إن تعذّرت قراءة الصورة (مثل HEIC)
    pub fn analyze(&self, path: &Path) -> Option<Vec<Embedding>> {
        let img = load_image(path)?;
        Some(
            self.detect(&img)
                .into_iter()
                .filter(|d| d.bbox[2].min(d.bbox[3]) >= MIN_FACE)
                .filter_map(|d| self.embed(&Self::align(&img, &d.landmarks)))
                .collect(),
        )
    }
}

fn bilinear(img: &RgbImage, x: f32, y: f32) -> image::Rgb<u8> {
    let (w, h) = (img.width() as i64, img.height() as i64);
    let (x0, y0) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let get = |xx: i64, yy: i64| -> [f32; 3] {
        if xx < 0 || yy < 0 || xx >= w || yy >= h {
            [0.0; 3]
        } else {
            let p = img.get_pixel(xx as u32, yy as u32);
            [p[0] as f32, p[1] as f32, p[2] as f32]
        }
    };
    let (p00, p10, p01, p11) = (get(x0, y0), get(x0 + 1, y0), get(x0, y0 + 1), get(x0 + 1, y0 + 1));
    let mut px = [0u8; 3];
    for c in 0..3 {
        let top = p00[c] * (1.0 - fx) + p10[c] * fx;
        let bottom = p01[c] * (1.0 - fx) + p11[c] * fx;
        px[c] = (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8;
    }
    image::Rgb(px)
}

// ---------- تعلّم الأشخاص من مجلداتهم ومطابقة الوجوه الجديدة ----------

/// تشابه يُعتبر عنده الوجهان لنفس الشخص (قيمة OpenCV الموصى بها لـ SFace)
pub const SAME_PERSON: f32 = 0.363;
/// حد قبول الاقتراح: متوسط أقرب 3 وجوه مرجعية
pub const MATCH: f32 = 0.42;
/// يجب أن يتفوق أفضل شخص على الثاني بهذا الفرق، وإلا فالصورة غير مؤكدة
pub const MARGIN: f32 = 0.05;

/// مجلد اسمه سنة أو اسم عام ليس مجلد شخص ("الصور من 2016"، "بلا عنوان"، "الأرشيف"، "Edit")
pub fn is_generic_folder_name(name: &str) -> bool {
    // بدون تشكيل: "المجلد المُقفَل" = "المجلد المقفل"
    let lower = crate::normalizer::normalize_arabic(&crate::normalizer::strip_diacritics(&crate::normalizer::to_western_digits(&name.to_lowercase())));
    let has_year = lower
        .split(|c: char| !c.is_ascii_digit())
        .any(|d| d.len() == 4 && (d.starts_with("19") || d.starts_with("20")));
    let words: String = lower.chars().filter(|c| c.is_alphabetic() || c.is_whitespace()).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
    const GENERIC: [&str; 22] = [
        "بلا عنوان", "الارشيف", "ارشيف", "مجلد جديد", "المجلد المقفل", "المجلد المُقفل", "صور", "الصور", "منوعات", "متفرقات",
        "edit", "edits", "untitled", "new folder", "archive", "camera", "camera roll", "screenshots", "whatsapp images", "downloads", "misc", "photos",
    ];
    // مجلدات خاصة أو مخفية من الجوال لا تُنقل إليها صور أبداً
    if words.contains("locked") || words.contains("private") || words.contains("hidden") || words.contains("خاص") {
        return true;
    }
    has_year || words.is_empty() || GENERIC.contains(&words.as_str()) || words.contains("مقفل")
}

#[derive(Debug, Clone)]
pub struct Person {
    pub name: String,
    pub dir: PathBuf,
    pub refs: Vec<Embedding>,
}

fn normalized_mean(vectors: &[&Embedding]) -> Embedding {
    let mut mean = vec![0.0f32; vectors.first().map_or(0, |v| v.len())];
    for v in vectors {
        for (m, x) in mean.iter_mut().zip(v.iter()) {
            *m += x;
        }
    }
    let norm = mean.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    mean.into_iter().map(|x| x / norm).collect()
}

/// هل المجلد لشخص واحد؟ الوجه المتكرر في أغلب الصور هو صاحب المجلد:
/// - يُحدد من الصور التي فيها وجه واحد، ثم يُضاف من الصور الجماعية الوجه الأقرب إليه
/// - يُقبل المجلد إذا ظهر هذا الوجه في أكثر من نصف الصور التي فيها وجوه
/// مجلدات السنوات والمناسبات (أشخاص مختلفون) تُرفض لأن لا وجه يتكرر في أغلبها.
pub fn learn_person(name: &str, dir: &Path, photos: &[Option<Vec<Embedding>>]) -> Option<Person> {
    let with_faces: Vec<&Vec<Embedding>> = photos.iter().flatten().filter(|f| !f.is_empty()).collect();
    let all: Vec<&Embedding> = with_faces.iter().flat_map(|f| f.iter()).collect();
    // مجلد صغير جداً لا يكفي لتعلّم وجه بثقة
    if with_faces.len() < 10 {
        return None;
    }
    // الوجه الذي له أكثر الأشباه بين كل الوجوه هو المرشح لصاحب المجلد
    let neighbours = |e: &Embedding| all.iter().filter(|o| cosine(e, o) >= SAME_PERSON).count();
    let anchor = *all.iter().max_by_key(|e| neighbours(e))?;
    let core: Vec<&Embedding> = all.iter().copied().filter(|o| cosine(anchor, o) >= SAME_PERSON).collect();
    let center = normalized_mean(&core);

    // من كل صورة: الوجه الأقرب لصاحب المجلد إن كان شبيهاً به
    let refs: Vec<Embedding> = with_faces
        .iter()
        .filter_map(|faces| faces.iter().max_by(|a, b| cosine(a, &center).total_cmp(&cosine(b, &center))))
        .filter(|f| cosine(f, &center) >= SAME_PERSON)
        .cloned()
        .collect();
    (refs.len() >= 5 && refs.len() * 2 > with_faces.len()).then(|| Person { name: name.to_string(), dir: dir.to_path_buf(), refs })
}

/// حذف الوجوه المرجعية الملتبسة: وجه في مجلد شخص يشبه شخصاً آخر أكثر من صاحب مجلده
/// (مثل صورة الأخ في مجلد أخيه). ويُحذف الشخص إن لم يبقَ له مرجع كافٍ.
pub fn remove_conflicts(people: &mut Vec<Person>) {
    let snapshot = people.clone();
    for (i, person) in people.iter_mut().enumerate() {
        person.refs.retain(|r| {
            let own = top_mean(r, &snapshot[i].refs, true);
            snapshot.iter().enumerate().filter(|(j, _)| *j != i).all(|(_, other)| top_mean(r, &other.refs, false) + MARGIN < own)
        });
    }
    people.retain(|p| p.refs.len() >= 5);
}

/// متوسط أعلى 3 تشابهات (skip_self: تجاهل الوجه نفسه إن كان من نفس المراجع)
fn top_mean(face: &Embedding, refs: &[Embedding], skip_self: bool) -> f32 {
    let mut sims: Vec<f32> = refs.iter().map(|r| cosine(face, r)).filter(|s| !(skip_self && *s > 0.9999)).collect();
    sims.sort_by(|a, b| b.total_cmp(a));
    let top = &sims[..sims.len().min(3)];
    top.iter().sum::<f32>() / top.len().max(1) as f32
}

fn person_score(face: &Embedding, person: &Person) -> f32 {
    top_mean(face, &person.refs, false)
}

/// من صاحب هذا الوجه؟ فقط إذا كان التشابه واضحاً ومتفوقاً على غيره
pub fn identify(face: &Embedding, people: &[Person], threshold: f32) -> Option<(usize, f32)> {
    let mut scores: Vec<(usize, f32)> = people.iter().enumerate().map(|(i, p)| (i, person_score(face, p))).collect();
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    let best = *scores.first()?;
    let second = scores.get(1).map_or(0.0, |s| s.1);
    (best.1 >= threshold && best.1 - second >= MARGIN).then_some(best)
}

/// صاحب الصورة: شخص معروف واحد فقط فيها (صورة فيها شخصان معروفان تبقى مكانها)
pub fn photo_owner(faces: &[Embedding], people: &[Person], threshold: f32) -> Option<(usize, f32)> {
    let found: Vec<(usize, f32)> = faces.iter().filter_map(|f| identify(f, people, threshold)).collect();
    let first = *found.first()?;
    found.iter().all(|(p, _)| *p == first.0).then(|| (first.0, found.iter().map(|f| f.1).fold(0.0, f32::max)))
}

// ---------- ذاكرة البصمات حتى لا يُعاد تحليل نفس الصورة ----------

#[derive(Serialize, Deserialize, Clone)]
struct CacheEntry {
    size: u64,
    modified: u64,
    /// None = تعذّرت قراءة الصورة
    faces: Option<Vec<Embedding>>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct FaceCache {
    entries: HashMap<String, CacheEntry>,
    #[serde(skip)]
    changed: bool,
}

fn file_stamp(path: &Path) -> Option<(u64, u64)> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some((meta.len(), modified))
}

impl FaceCache {
    pub fn load(path: &Path) -> Self {
        fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        if !self.changed {
            return;
        }
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_vec(self) {
            let tmp = path.with_extension("tmp");
            if fs::write(&tmp, text).is_ok() {
                let _ = fs::rename(&tmp, path);
            }
        }
    }

    fn get(&self, path: &Path) -> Option<Option<Vec<Embedding>>> {
        let (size, modified) = file_stamp(path)?;
        self.entries
            .get(&path.to_string_lossy().to_string())
            .filter(|e| e.size == size && e.modified == modified)
            .map(|e| e.faces.clone())
    }

    fn put(&mut self, path: &Path, faces: Option<Vec<Embedding>>) {
        if let Some((size, modified)) = file_stamp(path) {
            self.entries.insert(path.to_string_lossy().to_string(), CacheEntry { size, modified, faces });
            self.changed = true;
        }
    }
}

/// تحليل مجموعة صور على كل أنوية المعالج، مع الذاكرة. progress يزيد مع كل صورة.
pub fn analyze_all(paths: &[PathBuf], cache: &Mutex<FaceCache>, progress: &AtomicUsize) -> Result<HashMap<PathBuf, Option<Vec<Embedding>>>, String> {
    let mut results: HashMap<PathBuf, Option<Vec<Embedding>>> = HashMap::new();
    let mut todo: Vec<PathBuf> = Vec::new();
    {
        let cache = cache.lock().unwrap();
        for p in paths {
            match cache.get(p) {
                Some(faces) => {
                    results.insert(p.clone(), faces);
                    progress.fetch_add(1, Ordering::Relaxed);
                }
                None => todo.push(p.clone()),
            }
        }
    }
    if todo.is_empty() {
        return Ok(results);
    }
    let engine = engine()?;
    let analyzed: Vec<(PathBuf, Option<Vec<Embedding>>)> = todo
        .par_iter()
        .map(|p| {
            let faces = engine.analyze(p);
            progress.fetch_add(1, Ordering::Relaxed);
            (p.clone(), faces)
        })
        .collect();
    let mut cache = cache.lock().unwrap();
    for (p, faces) in analyzed {
        cache.put(&p, faces.clone());
        results.insert(p, faces);
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn year_generic_and_private_folders_are_not_people() {
        for name in ["الصور من 2016", "بلا عنوان", "بلا عنوان(1)", "الأرشيف", "Edit", "المجلد المُقفَل", "Locked Folder", "2019"] {
            assert!(is_generic_folder_name(name), "{name}");
        }
        for name in ["Ali", "Mom", "الخال", "جاك", "Hamzah", "الحمزة بحضن أبوه"] {
            assert!(!is_generic_folder_name(name), "{name}");
        }
    }

    #[test]
    fn models_load_and_blank_image_has_no_face() {
        let engine = engine().unwrap();
        let blank = RgbImage::from_pixel(300, 200, image::Rgb([128, 128, 128]));
        assert!(engine.detect(&blank).is_empty());
    }

    #[test]
    fn alignment_maps_template_points_to_themselves() {
        // صورة 112×112 نقاطها هي القالب نفسه: المحاذاة تعيد الصورة كما هي تقريباً
        let img = RgbImage::from_fn(112, 112, |x, y| image::Rgb([x as u8, y as u8, 0]));
        let aligned = FaceEngine::align(&img, &TEMPLATE);
        let p = aligned.get_pixel(50, 60);
        assert!((p[0] as i32 - 50).abs() <= 1 && (p[1] as i32 - 60).abs() <= 1, "{p:?}");
    }

    #[test]
    fn unreadable_images_are_reported_not_crashed() {
        let root = crate::organizer::tests::temp_tree("faces_bad", &["empty.jpg"]);
        assert!(load_image(&root.join("empty.jpg")).is_none());
        fs::remove_dir_all(&root).unwrap();
    }
}
