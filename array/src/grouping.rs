//! تجميع المقاطع الصوتية بمقارنة عدة أدلة بين الملفات، لا بدليل واحد.
//!
//! لكل ملف تُجمع الأدلة: أسماء الأشخاص (من الفنان، فنان الألبوم، الملحن، الألبوم، العنوان،
//! اسم الملف، واسم مجلد الفنان الموجود)، الألبوم، موقع/بريد الرفع، علامات الذكاء الاصطناعي،
//! المجلد الأصلي، بداية العنوان، ووقت التنزيل. ثم يُعطى كل زوج من الملفات نقاطاً على كل
//! دليل مشترك، ويُجمعان فقط إذا بلغت النقاط حداً يحتاج أكثر من دليل ضعيف.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;
use std::time::{Duration, SystemTime};

use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, ItemValue};
use regex::Regex;

use crate::normalizer::{fix_mojibake, normalize_arabic, strip_diacritics, to_western_digits};

/// مجلد الملفات المنفردة (لا يُنشأ مجلد لملف واحد أبداً)
pub const MISC_FOLDER: &str = "منوعات";
/// مجلد الملفات المصنوعة بالذكاء الاصطناعي
pub const AI_FOLDER: &str = "ذكاء اصطناعي";

/// كلمات تدل على أن الملف صُنع بأداة ذكاء اصطناعي (تُبحث في الاسم والمسار والوسوم)
pub const DEFAULT_AI_MARKERS: [&str; 12] = [
    "gemini", "suno", "udio", "riffusion", "mubert", "aiva", "boomy", "soundraw", "ai music", "ai song",
    "ذكاء اصطناعي", "الذكاء الاصطناعي",
];

/// أسماء مجلدات موجودة تُعاد استخدامها لمجلد المنوعات
const MISC_SYNONYMS: [&str; 5] = ["منوعات", "منوع", "متفرقات", "misc", "various"];
/// مجلدات النسخ القديمة للملفات غير المصنفة: لا تُعتبر اسم فنان، وتُفرَّغ إلى المنوعات
const OLD_MISC: [&str; 2] = ["single tracks", "unknown"];

/// حد النقاط لجمع ملفين: اسم كامل مشترك يكفي، أما الأدلة الضعيفة فتحتاج ثلاثة
const GROUP_THRESHOLD: u32 = 3;
/// دلو أكبر من هذا (مثل مجلد فيه آلاف الملفات) لا يُستخدم لتوليد الأزواج لأنه لا يميّز
const MAX_WEAK_BUCKET: usize = 400;
const CLOSE_DOWNLOAD: Duration = Duration::from_secs(15 * 60);

/// كلمات ألقاب وقنوات تُحذف عند مقارنة الأسماء (بعد التطبيع: ة←ه، ى←ي، أ←ا)
const TITLE_WORDS: [&str; 41] = [
    "الحلقه", "حلقه", "episode", "ep", "part", "track", "audio",
    "قناه", "القارئ", "قارئ", "للقارئ", "الشيخ", "شيخ", "للشيخ", "فضيله", "الفنان", "فنان", "للفنان",
    "المنشد", "منشد", "للمنشد", "الدكتور", "دكتور", "د", "الامام", "الداعيه", "الاستاذ", "بصوت", "تلاوه",
    "channel", "official", "topic", "sheikh", "shaikh", "qari", "reciter", "dr", "the", "by", "ft", "feat",
];

/// كلمات بعدها يأتي اسم الشخص في العنوان (مثل: "سورة مريم بصوت الشيخ عبدالله الموسى")
const NAME_TRIGGERS: [&str; 16] = [
    "الشيخ", "للشيخ", "القارئ", "للقارئ", "الفنان", "للفنان", "المنشد", "للمنشد", "الدكتور", "بصوت",
    "الداعيه", "فضيله", "الامام", "sheikh", "qari", "by",
];

/// وصلات داخل الأسماء لا تُعد اسماً مستقلاً (علي بن عبدالخالق القرني)
const NAME_CONNECTORS: [&str; 4] = ["بن", "ابن", "بنت", "ال"];

/// ألبومات عامة لا تربط بين الملفات (تُقارن بعد person_key)
const GENERIC_ALBUMS: [&str; 12] = [
    "music", "موسيقي", "audio", "songs", "song", "اغاني", "اغنيه", "unknown", "unknown album", "mp3", "youtube", "single",
];

/// أسماء لا تدل على شخص
const GENERIC_NAMES: [&str; 8] = ["unknown", "unknown artist", "various artists", "artist", "فنان", "مجهول", "track", "audio"];

static RE_SOURCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)[\w.+-]+@[\w-]+\.[\w.]+|(?:https?://)?(?:www\.)?([\w-]+\.(?:com|net|org|info|me|tv|sa|io))\b").unwrap());

#[derive(Debug, Clone)]
pub struct Mention {
    pub display: String,
    pub key: String,
    pub from_tags: bool,
}

/// أدلة ملف صوتي واحد
#[derive(Debug, Default, Clone)]
pub struct Track {
    pub mentions: Vec<Mention>,
    pub album: Option<(String, String)>,
    pub sources: Vec<(String, String)>,
    pub ai: bool,
    /// اسم مجلد فنان موجود داخل مجلد الموسيقى يقع فيه الملف حالياً
    pub artist_folder: Option<String>,
    /// المجلد الأصلي للملف (خارج مجلد الموسيقى)، إن لم يكن عاماً
    pub folder: Option<(String, String)>,
    pub title_prefix: Option<(String, String)>,
    pub modified: Option<SystemTime>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Assignment {
    /// اسم المجلد النهائي داخل مجلد الموسيقى (موجود أو جديد)
    Folder(String),
    Ai,
    Misc,
}

/// مفتاح مقارنة الأسماء: تطبيع، حذف التشكيل والألقاب والأرقام، ودمج "عبد الله" → "عبدالله"
pub fn person_key(text: &str) -> String {
    let text = to_western_digits(&strip_diacritics(&normalize_arabic(&fix_mojibake(text)))).to_lowercase();
    let text = text.replace("عبد ال", "عبدال").replace("ابو ", "ابو").replace("أبو ", "ابو");
    let cleaned: String = text.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    cleaned
        .split_whitespace()
        .filter(|w| !TITLE_WORDS.contains(w) && !w.chars().all(|c| c.is_ascii_digit()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn word_key(word: &str) -> String {
    person_key(word)
}

fn is_source_like(text: &str) -> bool {
    let lower = text.to_lowercase();
    RE_SOURCE.is_match(text)
        || ["http", "www.", "تم رفع", "بواسطة", "مجموعة مواقع", "حصريا على"].iter().any(|m| lower.contains(m))
}

/// مفتاح موقع/بريد الرفع: النطاق أو البريد إن وُجد، وإلا النص المطبَّع
fn source_key(text: &str) -> (String, String) {
    if let Some(m) = RE_SOURCE.captures(text) {
        let key = m.get(1).map_or(m.get(0).unwrap().as_str(), |d| d.as_str()).to_lowercase();
        return (key.clone(), key);
    }
    (text.trim().to_string(), person_key(text))
}

fn split_names(text: &str) -> Vec<String> {
    text.split(['\0', ';', '/', '|', '｜', '،', ',', '&', '+'])
        .map(|s| s.trim().trim_matches('-').trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// إزالة كلمات القنوات من الاسم المعروض ("قناة القارئ ..." → "القارئ ...")
fn clean_display(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let kept: Vec<&str> = words
        .into_iter()
        .filter(|w| !matches!(normalize_arabic(&w.to_lowercase()).as_str(), "قناه" | "channel" | "topic" | "official"))
        .filter(|w| *w != "-")
        .collect();
    kept.join(" ").trim().trim_matches('-').trim().to_string()
}

fn mention(display: &str, from_tags: bool) -> Option<Mention> {
    let display = clean_display(&fix_mojibake(display));
    let key = person_key(&display);
    let letters = key.chars().filter(|c| c.is_alphabetic()).count();
    if letters < 3 || GENERIC_NAMES.contains(&key.as_str()) || display.to_uppercase().starts_with("AUD-") {
        return None;
    }
    Some(Mention { display, key, from_tags })
}

/// الأسماء التي تأتي بعد لقب في النص: "تلاوة رائعة الشيخ عبدالله الموسى" → "عبدالله الموسى"
fn names_after_titles(text: &str) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < words.len() {
        if NAME_TRIGGERS.contains(&word_key(words[i]).as_str()) || NAME_TRIGGERS.contains(&normalize_arabic(words[i]).as_str()) {
            let mut j = i + 1;
            // تخطّي ألقاب متتالية ("بصوت الشيخ")
            while j < words.len() && TITLE_WORDS.contains(&normalize_arabic(&words[j].to_lowercase()).as_str()) {
                j += 1;
            }
            let mut name = Vec::new();
            let mut name_words = 0;
            let mut after_connector = false;
            while j < words.len() && name_words < 2 && name.len() < 6 {
                let w = words[j];
                if !w.chars().any(|c| c.is_alphabetic()) {
                    break;
                }
                let k = normalize_arabic(&w.to_lowercase());
                name.push(w);
                // "عبد" و"ابو" تُدمج مع ما بعدها، والاسم بعد "بن" جزء من النسب: لا يُحسب أي منها
                let connector = NAME_CONNECTORS.contains(&k.as_str()) || k == "عبد" || k == "ابو";
                if !connector && !after_connector {
                    name_words += 1;
                }
                after_connector = NAME_CONNECTORS.contains(&k.as_str());
                j += 1;
            }
            if name_words >= 2 {
                found.push(name.join(" "));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    found
}

fn first_words_key(text: &str) -> Option<(String, String)> {
    let words: Vec<&str> = text
        .split_whitespace()
        .filter(|w| w.chars().any(|c| c.is_alphabetic()))
        .take(2)
        .collect();
    if words.len() < 2 {
        return None;
    }
    let display = words.join(" ");
    let key = person_key(&display);
    (key.split_whitespace().count() >= 2).then_some((display, key))
}

/// العلامة تُطابق كلمات كاملة فقط ("udio" لا تُطابق داخل "audio")
fn contains_marker(text: &str, markers: &[String]) -> bool {
    let words = |t: &str| -> String {
        let cleaned: String = normalize_arabic(&t.to_lowercase()).chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
        format!(" {} ", cleaned.split_whitespace().collect::<Vec<_>>().join(" "))
    };
    let text = words(text);
    markers.iter().map(|m| words(m)).any(|m| m.trim().len() > 0 && text.contains(&m))
}

fn folder_kind(name: &str) -> (bool, bool) {
    let key = person_key(name);
    let lower = name.trim().to_lowercase();
    let is_misc = MISC_SYNONYMS.iter().any(|m| person_key(m) == key) || OLD_MISC.contains(&lower.as_str());
    let is_ai = person_key(AI_FOLDER) == key;
    (is_misc, is_ai)
}

/// قراءة أدلة ملف صوتي. artist و title من اسم الملف/الوسوم (OrganizerEngine::extract_audio_info)،
/// و speaker ما استخرجه منطق الحلقات/أصحاب المقاطع إن وُجد.
pub fn read_track(
    path: &Path,
    root: &Path,
    music_dir: &Path,
    file_artist: &str,
    title: &str,
    speaker: Option<&str>,
    ai_markers: &[String],
) -> Track {
    let mut track = Track::default();
    let markers: Vec<String> = ai_markers.iter().map(|m| normalize_arabic(&m.to_lowercase())).collect();
    let stem = fix_mojibake(&path.file_stem().unwrap_or_default().to_string_lossy());
    let mut all_text: Vec<String> = vec![stem.clone()];

    // الوسوم
    if let Ok(tagged) = Probe::open(path).and_then(|p| p.read()) {
        for tag in tagged.tags() {
            for item in tag.items() {
                let raw = match item.value() {
                    ItemValue::Text(t) | ItemValue::Locator(t) => t,
                    ItemValue::Binary(_) => continue,
                };
                let text = fix_mojibake(raw);
                if text.trim().is_empty() {
                    continue;
                }
                all_text.push(text.clone());
                if is_source_like(&text) {
                    let source = source_key(&text);
                    if !track.sources.iter().any(|(_, k)| *k == source.1) {
                        track.sources.push(source);
                    }
                }
                match item.key() {
                    ItemKey::TrackArtist | ItemKey::AlbumArtist | ItemKey::Composer | ItemKey::OriginalArtist => {
                        if !is_source_like(&text) {
                            track.mentions.extend(split_names(&text).iter().filter_map(|n| mention(n, true)));
                            // الاسم كاملاً أيضاً، مثل "قناة القارئ; عبدالله الموسى"
                            track.mentions.extend(mention(&text.replace(['\0', ';', '/'], " "), true));
                        }
                    }
                    ItemKey::AlbumTitle => {
                        let key = person_key(&text);
                        if GENERIC_ALBUMS.contains(&key.as_str()) {
                            continue;
                        }
                        if !key.is_empty() {
                            track.album = Some((text.trim().to_string(), key));
                        }
                        // ألبوم قصير بلا أرقام ولا مواقع غالباً اسم شخص ("علي القرني")
                        if !is_source_like(&text) && text.split_whitespace().count() <= 4 && !text.chars().any(|c| c.is_ascii_digit()) {
                            track.mentions.extend(split_names(&text).iter().filter_map(|n| mention(n, true)));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // أسماء من العنوان واسم الملف
    for text in [title.to_string(), stem.clone()] {
        for name in names_after_titles(&fix_mojibake(&text)) {
            track.mentions.extend(mention(&name, false));
        }
    }
    if !file_artist.trim().is_empty() {
        track.mentions.extend(mention(file_artist, false));
    }
    if let Some(s) = speaker {
        track.mentions.extend(mention(s, false));
    }

    // المجلد الحالي
    if let Some(parent) = path.parent() {
        let name = fix_mojibake(&parent.file_name().unwrap_or_default().to_string_lossy());
        let (is_misc, is_ai) = folder_kind(&name);
        if parent.parent() == Some(music_dir) && !is_misc {
            if is_ai {
                track.ai = true;
            } else {
                track.artist_folder = Some(parent.file_name().unwrap_or_default().to_string_lossy().to_string());
                track.mentions.extend(mention(&name, true));
            }
        } else if parent != root && parent != music_dir && !is_misc {
            let key = person_key(&name);
            if !key.is_empty() {
                track.folder = Some((name.clone(), key));
            }
        }
    }

    // علامات الذكاء الاصطناعي في الاسم والمسار (من المجلد المختار) والوسوم
    let relative = path.strip_prefix(root).unwrap_or(path).to_string_lossy().to_string();
    all_text.push(fix_mojibake(&relative));
    if all_text.iter().any(|t| contains_marker(t, &markers)) {
        track.ai = true;
    }

    track.title_prefix = first_words_key(if title.trim().is_empty() { &stem } else { title });
    track.modified = fs::metadata(path).and_then(|m| m.modified()).ok();
    track
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

fn union(parent: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        parent[ra.max(rb)] = ra.min(rb);
    }
}

/// هل الاسمان لشخص واحد؟ متطابقان، أو كلمات الأقصر (اسمان على الأقل) كلها في الأطول،
/// أو الأقصر (9 أحرف فأكثر) موجود داخل الأطول بدون مسافات ("بدرالحميدي" ↔ "بدر الحميدي")
fn same_person(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let short_words: Vec<&str> = short.split_whitespace().collect();
    let long_words: HashSet<&str> = long.split_whitespace().collect();
    if short_words.len() >= 2 && short_words.iter().all(|w| long_words.contains(w)) {
        return true;
    }
    let short_compact: String = short.split_whitespace().collect();
    let long_compact: String = long.split_whitespace().collect();
    short_compact.chars().count() >= 9 && long_compact.contains(&short_compact)
}

/// اسم كامل (اسمان على الأقل أو اسم طويل) دليل قوي يكفي وحده
fn is_full_name(key: &str) -> bool {
    key.split_whitespace().count() >= 2 || key.chars().count() >= 9
}

struct People {
    /// لكل ذكر في كل ملف: رقم الشخص الموحَّد
    track_people: Vec<Vec<(usize, bool)>>,
    /// لكل شخص: عدد الملفات التي ذكرته، أفضل اسم للعرض، ومفتاحه
    count: Vec<usize>,
    display: Vec<String>,
    key: Vec<String>,
}

fn unify_people(tracks: &[Track]) -> People {
    let mut keys: Vec<String> = tracks.iter().flat_map(|t| t.mentions.iter().map(|m| m.key.clone())).collect();
    keys.sort();
    keys.dedup();
    let mut parent: Vec<usize> = (0..keys.len()).collect();
    for i in 0..keys.len() {
        for j in i + 1..keys.len() {
            if same_person(&keys[i], &keys[j]) {
                union(&mut parent, i, j);
            }
        }
    }
    let index: HashMap<&str, usize> = keys.iter().enumerate().map(|(i, k)| (k.as_str(), i)).collect();
    let mut roots: HashMap<usize, usize> = HashMap::new();
    let mut track_people = Vec::with_capacity(tracks.len());
    let mut count: Vec<usize> = Vec::new();
    let mut displays: Vec<HashMap<String, (usize, usize)>> = Vec::new();
    let mut key: Vec<String> = Vec::new();

    for t in tracks {
        let mut seen: HashMap<usize, bool> = HashMap::new();
        for m in &t.mentions {
            let root = find(&mut parent, index[m.key.as_str()]);
            let next = roots.len();
            let id = *roots.entry(root).or_insert(next);
            if id == count.len() {
                count.push(0);
                displays.push(HashMap::new());
                key.push(m.key.clone());
            }
            let from_tags = seen.entry(id).or_insert(false);
            *from_tags |= m.from_tags;
            let entry = displays[id].entry(m.display.clone()).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += m.from_tags as usize;
            // أقصر مفتاح كامل يمثل الشخص (بدون ألقاب ولا إضافات)
            if is_full_name(&m.key) && (m.key.len() < key[id].len() || !is_full_name(&key[id])) {
                key[id] = m.key.clone();
            }
        }
        for id in seen.keys() {
            count[*id] += 1;
        }
        track_people.push(seen.into_iter().collect());
    }

    // اسم العرض: الأكثر تكراراً، ثم الأكثر وروداً في الوسوم، ثم الأقصر
    let display = displays
        .into_iter()
        .map(|d| {
            d.into_iter()
                .max_by(|(a_name, a), (b_name, b)| a.cmp(b).then(b_name.chars().count().cmp(&a_name.chars().count())))
                .map(|(name, _)| name)
                .unwrap_or_default()
        })
        .collect();
    People { track_people, count, display, key }
}

/// الشخص الأساسي لكل ملف: الأكثر وروداً بين كل الملفات (فاسم الشيخ يغلب اسم القناة التي رفعت ملفاً واحداً)
fn primary_people(people: &People) -> Vec<Option<usize>> {
    people
        .track_people
        .iter()
        .map(|list| {
            list.iter()
                .filter(|(id, _)| people.count[*id] >= 2)
                .max_by_key(|(id, from_tags)| (people.count[*id], *from_tags, people.key[*id].len()))
                .map(|(id, _)| *id)
        })
        .collect()
}

fn pair_score(a: &Track, b: &Track, pa: Option<usize>, pb: Option<usize>, people: &People) -> u32 {
    let mut score = 0;
    if let (Some(x), Some(y)) = (pa, pb) {
        if x == y {
            score += if is_full_name(&people.key[x]) { 3 } else { 2 };
        }
    }
    if a.ai && b.ai {
        score += 3;
    }
    if let (Some((_, x)), Some((_, y))) = (&a.album, &b.album) {
        if x == y {
            score += 2;
        }
    }
    if a.sources.iter().any(|(_, x)| b.sources.iter().any(|(_, y)| x == y)) {
        score += 1;
    }
    if let (Some((_, x)), Some((_, y))) = (&a.folder, &b.folder) {
        if x == y {
            score += 1;
        }
    }
    if let (Some(x), Some(y)) = (&a.artist_folder, &b.artist_folder) {
        if x == y {
            score += 1;
        }
    }
    if let (Some((_, x)), Some((_, y))) = (&a.title_prefix, &b.title_prefix) {
        if x == y {
            score += 1;
        }
    }
    if let (Some(x), Some(y)) = (a.modified, b.modified) {
        let gap = x.duration_since(y).or_else(|_| y.duration_since(x)).unwrap_or(Duration::MAX);
        if gap <= CLOSE_DOWNLOAD {
            score += 1;
        }
    }
    score
}

fn majority<T: Clone + Eq + std::hash::Hash>(items: impl Iterator<Item = T>) -> Option<(T, usize)> {
    let mut counts: HashMap<T, usize> = HashMap::new();
    let mut order: Vec<T> = Vec::new();
    for item in items {
        let c = counts.entry(item.clone()).or_insert(0);
        if *c == 0 {
            order.push(item);
        }
        *c += 1;
    }
    order.into_iter().map(|i| (i.clone(), counts[&i])).max_by_key(|(_, c)| *c)
}

/// اختيار مجلد موجود لاسم المجموعة: يُفضَّل المطابق تماماً، ثم ما ليس فيه اسم قناة، ثم الأقصر
fn existing_folder_for(name_key: &str, existing: &[String]) -> Option<String> {
    existing
        .iter()
        .filter(|dir| {
            let (is_misc, is_ai) = folder_kind(dir);
            let key = person_key(dir);
            !is_misc && !is_ai && !key.is_empty() && same_person(&key, name_key)
        })
        .min_by_key(|dir| {
            let key = person_key(dir);
            let channel = normalize_arabic(&dir.to_lowercase()).contains("قناه") || dir.to_lowercase().contains("channel");
            (key != name_key, channel, dir.chars().count())
        })
        .cloned()
}

/// توزيع الملفات على المجموعات. existing_dirs: أسماء المجلدات الموجودة داخل مجلد الموسيقى.
pub fn assign(tracks: &[Track], existing_dirs: &[String]) -> Vec<Assignment> {
    let n = tracks.len();
    let people = unify_people(tracks);
    let primary = primary_people(&people);

    // أزواج مرشحة: ملفات تشترك في دليل واحد على الأقل
    let mut buckets: HashMap<(u8, String), Vec<usize>> = HashMap::new();
    for (i, t) in tracks.iter().enumerate() {
        if let Some(p) = primary[i] {
            buckets.entry((0, p.to_string())).or_default().push(i);
        }
        if t.ai {
            buckets.entry((1, String::new())).or_default().push(i);
        }
        if let Some((_, k)) = &t.album {
            buckets.entry((2, k.clone())).or_default().push(i);
        }
        for (_, k) in &t.sources {
            buckets.entry((3, k.clone())).or_default().push(i);
        }
        if let Some((_, k)) = &t.folder {
            buckets.entry((4, k.clone())).or_default().push(i);
        }
        if let Some(k) = &t.artist_folder {
            buckets.entry((5, k.clone())).or_default().push(i);
        }
        if let Some((_, k)) = &t.title_prefix {
            buckets.entry((6, k.clone())).or_default().push(i);
        }
    }

    let mut parent: Vec<usize> = (0..n).collect();
    let mut checked: HashSet<(usize, usize)> = HashSet::new();
    for ((kind, _), members) in &buckets {
        let strong = *kind <= 2;
        if members.len() < 2 || (!strong && members.len() > MAX_WEAK_BUCKET) {
            continue;
        }
        for (x, &i) in members.iter().enumerate() {
            for &j in &members[x + 1..] {
                if find(&mut parent, i) == find(&mut parent, j) || !checked.insert((i, j)) {
                    continue;
                }
                if pair_score(&tracks[i], &tracks[j], primary[i], primary[j], &people) >= GROUP_THRESHOLD {
                    union(&mut parent, i, j);
                }
            }
        }
    }

    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(i);
    }

    let mut result = vec![Assignment::Misc; n];
    for members in groups.values() {
        if members.len() < 2 {
            continue; // ملف منفرد → منوعات
        }
        let ai_count = members.iter().filter(|&&i| tracks[i].ai).count();
        let assignment = if ai_count * 2 >= members.len() {
            Assignment::Ai
        } else if let Some((Some(person), _)) = majority(members.iter().map(|&i| primary[i])) {
            let name = existing_folder_for(&people.key[person], existing_dirs).unwrap_or_else(|| people.display[person].clone());
            Assignment::Folder(name)
        } else if let Some((Some(dir), c)) = majority(members.iter().map(|&i| tracks[i].artist_folder.clone())) {
            if c * 2 >= members.len() { Assignment::Folder(dir) } else { Assignment::Misc }
        } else if let Some((Some((name, _)), _)) = majority(members.iter().map(|&i| tracks[i].album.clone()).filter(|a| a.as_ref().is_none_or(|(n, _)| !is_source_like(n)))) {
            Assignment::Folder(name)
        } else if let Some((Some((name, _)), _)) = majority(members.iter().map(|&i| tracks[i].folder.clone())) {
            Assignment::Folder(name)
        } else if let Some((Some((name, _)), _)) = majority(members.iter().map(|&i| tracks[i].sources.first().cloned())) {
            Assignment::Folder(name)
        } else if let Some((Some((name, _)), _)) = majority(members.iter().map(|&i| tracks[i].title_prefix.clone())) {
            Assignment::Folder(name)
        } else {
            Assignment::Misc
        };
        let assignment = match assignment {
            // اسم المجلد ينظَّف مثل أسماء الملفات: ترميز مشوّه، "[AUD"، "(1)"...
            Assignment::Folder(name) => Assignment::Folder(crate::organizer::OrganizerEngine::clean_string(&name)),
            other => other,
        };
        for &i in members {
            result[i] = assignment.clone();
        }
    }
    result
}

/// اسم مجلد المنوعات: مجلد موجود يناسبه، وإلا "منوعات"
pub fn misc_folder_name(existing_dirs: &[String]) -> String {
    MISC_SYNONYMS
        .iter()
        .find_map(|syn| existing_dirs.iter().find(|d| person_key(d) == person_key(syn)).cloned())
        .unwrap_or_else(|| MISC_FOLDER.to_string())
}

/// اسم مجلد الذكاء الاصطناعي: الموجود إن وُجد
pub fn ai_folder_name(existing_dirs: &[String]) -> String {
    existing_dirs
        .iter()
        .find(|d| folder_kind(d).1)
        .cloned()
        .unwrap_or_else(|| AI_FOLDER.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(display: &str, from_tags: bool) -> Mention {
        mention(display, from_tags).unwrap()
    }

    fn track(mentions: Vec<Mention>) -> Track {
        Track { mentions, ..Track::default() }
    }

    #[test]
    fn person_keys_ignore_titles_channels_and_spelling() {
        assert_eq!(person_key("قناة القارئ;  عبدالله الموسى"), person_key("القارئ عبد الله الموسي"));
        assert_eq!(person_key("الشيخ علي بن عبد الخالق القرني"), "علي بن عبدالخالق القرني");
        assert_eq!(person_key("أبوبكر سالم Abu Bakr Salem - Topic"), "ابوبكر سالم abu bakr salem");
    }

    #[test]
    fn same_person_rules() {
        assert!(same_person(&person_key("علي القرني"), &person_key("علي بن عبدالخالق القرني")));
        assert!(same_person(&person_key("بدر الحميدي"), &person_key("بدرالحميدي Badr al-Hamidi")));
        // اسم أول وحده لا يكفي للربط بين شخصين مختلفين
        assert!(!same_person(&person_key("عبدالله"), &person_key("عبدالله المخيلد")));
        assert!(!same_person(&person_key("عبدالله الموسى"), &person_key("عبدالله المخيلد")));
    }

    #[test]
    fn finds_names_after_titles_in_text() {
        assert_eq!(names_after_titles("سورة مريم تلاوة رائعه الشيخ عبدالله الموسى"), ["عبدالله الموسى"]);
        assert_eq!(names_after_titles("سورة الدخان بصوت الشيخ عبدالله الموسي"), ["عبدالله الموسي"]);
        assert_eq!(names_after_titles("الشام والعراق - الشيخ علي بن عبدالخالق القرني"), ["علي بن عبدالخالق القرني"]);
        assert!(names_after_titles("أغنية جميلة").is_empty());
    }

    /// حالة المستخدم الحقيقية: خمسة ملفات للقارئ نفسه بأدلة مختلفة في كل ملف
    #[test]
    fn groups_the_same_reciter_from_different_clues() {
        let tracks = vec![
            // مجلد قديم باسمه + وسم الفنان
            track(vec![m("القارئ عبدالله الموسى", true), m("القارئ عبدالله الموسى", true)]),
            // وسم الفنان فيه اسمان مدموجان + الألبوم
            track(vec![m("عبدالله الموسى", true), m("عبدالله الموسى", true)]),
            track(vec![m("عبدالله الموسى", true)]),
            // الفنان اسم قناة الرفع، والاسم في العنوان فقط
            track(vec![m("Ajmal_Alqeraat", true), m("عبدالله الموسى", false)]),
            // الفنان موقع، والاسم في الألبوم بتهجئة مختلفة
            track(vec![m("القارئ عبد الله الموسى", true), m("عبدالله الموسي", false)]),
        ];
        let existing = vec!["القارئ عبدالله الموسى".to_string(), "قناة القارئ عبدالله الموسى".to_string()];

        let result = assign(&tracks, &existing);

        assert!(result.iter().all(|a| *a == Assignment::Folder("القارئ عبدالله الموسى".to_string())), "{result:?}");
    }

    #[test]
    fn channel_words_alone_are_not_a_name() {
        assert!(mention("قناة القارئ", true).is_none());
        assert_eq!(mention("قناة القارئ عبدالله الموسى", true).unwrap().display, "القارئ عبدالله الموسى");
    }

    #[test]
    fn a_single_file_never_gets_its_own_folder() {
        let tracks = vec![track(vec![m("هاجر نعمان", true)]), track(vec![m("اسامه الرضي", true)])];
        assert_eq!(assign(&tracks, &[]), [Assignment::Misc, Assignment::Misc]);
    }

    #[test]
    fn ai_files_are_grouped_even_without_other_clues() {
        let ai = Track { ai: true, ..Track::default() };
        let tracks = vec![ai.clone(), ai, track(vec![m("هاجر نعمان", true)])];
        assert_eq!(assign(&tracks, &[]), [Assignment::Ai, Assignment::Ai, Assignment::Misc]);
    }

    #[test]
    fn one_weak_clue_is_not_enough() {
        let mut a = track(vec![]);
        let mut b = track(vec![]);
        a.folder = Some(("Downloads 2".into(), "downloads 2".into()));
        b.folder = a.folder.clone();
        assert_eq!(assign(&[a.clone(), b.clone()], &[]), [Assignment::Misc, Assignment::Misc]);

        // نفس المجلد + نفس الموقع + نفس وقت التنزيل = ثلاثة أدلة ضعيفة → تُجمع
        let now = SystemTime::now();
        a.sources = vec![("hocine200@live.com".into(), "hocine200@live.com".into())];
        b.sources = a.sources.clone();
        a.modified = Some(now);
        b.modified = Some(now + Duration::from_secs(60));
        assert_eq!(assign(&[a, b], &[]), [Assignment::Folder("Downloads 2".into()), Assignment::Folder("Downloads 2".into())]);
    }

    #[test]
    fn uploader_name_loses_to_the_real_person() {
        // "Albannaa" رفع ملفين، لكن اسم الشيخ موجود في ستة ملفات
        let qarni = |extra: &str| track(vec![m(extra, true), m("علي القرني", true)]);
        let tracks = vec![
            qarni("Albannaa"),
            qarni("Albannaa"),
            qarni("Alresala Islam"),
            qarni("Alresala Islam"),
            track(vec![m("الشيخ علي بن عبد الخالق القرني", true)]),
            track(vec![m("الشيخ علي بن عبد الخالق القرني", true)]),
        ];
        let result = assign(&tracks, &[]);
        assert!(result.iter().all(|a| matches!(a, Assignment::Folder(n) if n == "علي القرني")), "{result:?}");
    }

    #[test]
    fn ai_markers_match_whole_words_only() {
        let markers: Vec<String> = DEFAULT_AI_MARKERS.iter().map(|m| m.to_string()).collect();
        assert!(contains_marker("gemini music [AUD-d61dabc6]", &markers));
        assert!(contains_marker("Made with Suno", &markers));
        assert!(!contains_marker("audio track", &markers)); // udio داخل audio
        assert!(!contains_marker("Laiva song", &markers));
    }

    #[test]
    fn misc_and_ai_folder_names_reuse_existing() {
        assert_eq!(misc_folder_name(&["متفرقات".into()]), "متفرقات");
        assert_eq!(misc_folder_name(&["Single Tracks".into()]), MISC_FOLDER);
        assert_eq!(ai_folder_name(&[]), AI_FOLDER);
    }
}

