# تقرير المرحلة 0 — التشخيص (بلا كود)

الفرع: `plan/phase-0` (من `main`) — تاريخ التشغيل: 2026-11-17.

## ما تغيّر
- `DIAGNOSIS.md` — جديد: قائمة 31 ملاحظة مرقّمة من تشغيل فعلي، وهي معيار القبول للمراحل التالية.
- `REPORTS/phase-0/1-start.png` — شاشة البداية قبل اختيار مجلد.
- `REPORTS/phase-0/2-preview.png` — شاشة معاينة التغييرات (22 عملية) قبل التنفيذ.
- `REPORTS/phase-0/3-result.png` — شاشة النتيجة بعد الترتيب.
- `REPORTS/phase-0/4-undo-dialog.png` — تأكيد التراجع من شاشة النتيجة.
- `PLAN.md` و`AGENTS.md` و`IDEA.md` — كانت غير متتبَّعة (untracked) فأُضيفت في هذا الـ commit.
- **لا تغيير في كود `array/src`.** استُخدم اختبار مؤقت لجمع النتائج ثم أُزيل، و`git diff array/` نظيف.

## الاختبارات
قبل التشخيص وبعده (شجرة `main` بلا تعديل): **82 ناجح، 0 فاشل، 1 متجاهَل** من 83.
المتجاهَل الوحيد هو `icons::tests::export_icons` وهو مُصدِّر SVG متعمَّد (`--ignored`)، ليس اختبارًا مُعطَّلًا، ولم يُلمس.

آخر 15 سطرًا من `cargo test`:

```
test safety::tests::selected_folder_is_refused_when_it_is_a_program ... ok
test organizer::tests::reuses_existing_folders_by_arabic_or_english_name ... ok
test storage::tests::undo_log_round_trip_and_clear ... ok
test organizer::tests::never_touches_programs_projects_scripts_or_web_pages ... ok
test safety::tests::detects_program_and_project_folders ... ok
test organizer::tests::videos_go_to_their_year_and_live_photos_stay_with_their_photo ... ok
test organizer::tests::running_twice_changes_nothing ... ok
test video::tests::year_comes_from_the_video_metadata ... ok
test organizer::tests::organizes_a_folder_end_to_end ... ok
test organizer::tests::undo_restores_the_exact_original_tree ... ok
test safety::tests::hidden_desktop_ini_does_not_mark_a_folder ... ok
test faces::tests::models_load_and_blank_image_has_no_face ... ok

test result: ok. 82 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 2.16s
```

## التحقق اليدوي
- بُني `cargo build --release` (5m43s، بلا تحذير) ← `target/release/Array.exe` = 67,712,512 بايت (64.6 MiB).
- أُنشئ مجلد اختبار مؤقت `%LOCALAPPDATA%\Temp\array-diag` بـ24 ملفًا: صور مع EXIF وبدونه،
  `IMG-…-WA…`، لقطتا شاشة، `IMG_####`، فيديو داخلي التاريخ مع `.srt`، `heic+mov`، تسجيلات صوتية،
  أغنية بوسوم، `PDF` بعنوان عربي داخلي و`PDF` بدونه، `DOCX` بعنوان قالب و`DOCX` بعنوان صالح،
  `.zip`، `.exe` في الجذر وآخر في `tools\`، نسخة مكررة، ومجلد مستخدم `صور\قديمة`.
  (ملف `.heic` هنا بايتات JPEG باسم `.heic` لأن ويندوز لا يملك libheif — أثّر على قراءة الوسوم فقط،
  وقد ذُكر أثره في التشخيص.)
- **مساري تشغيل بغرض التحقق المزدوج:** (1) تشغيل المحرك على نسخ المجلد (default وrebuild)،
  (2) تشغيل الواجهة الحقيقية على `array-diag-gui`: اختيار المجلد ← «معاينة التغييرات» ← «رتّب 22 ملفًا» ←
  «تراجع (22 ملفًا)». الشجرتان الناتجتان متطابقتان، والتراجع أعاد 26 ملفًا ومجلدين إلى ما كانتا عليه
  (تحقق بـ`find` قبل وبعد).
- تشغيل إضافي على نفس المجلد مع `setup_tool.exe` في الجذر: **رُفض المجلد كله** برسالة
  «This folder belongs to a program (setup_tool.exe)» دون ترتيب أي ملف (التشخيص بند 17).
- زمن التخطيط: 129 مللي ثانية (افتراضي) و58 مللي ثانية (إعادة بناء) لـ24 ملفًا. لا مشكلة أداء في الترتيب.
- الصور في `REPORTS/phase-0/` — راجعتها بالعين: عربي RTL، أرقام غربية، لكن جدول المعاينة
  يقرأ المسارات مقلوبة (بند 28).

## ما لم يُنفَّذ أو لم يُتحقق منه، ولماذا
- **الترتيب بالوجوه لم يُختبر فعليًا** (لا مجلدات أشخاص فيها 10 صور فأكثر في المجلد التجريبي)،
  والمرحلة 1-أ ستزيله أصلًا. النماذج موجودة (38 MB) ومُحمَّلة في البناء.
- **مجموعة السحابة (OneDrive)** لم تُختبر: لا مجلد سحابي في مجلد الاختبار.
- **مجلد تنزيلات المستخدم الحقيقي (11 GB) لم يُستخدم**، التزامًا بقاعدة «لا تلمس ملفات المستخدم»؛
  العينات صُنعت في مجلد مؤقت. لذلك قد تظهر مشاكل إضافية على بيانات حقيقية (عدد كبير، أسماء عربية طويلة).
- **سرعة الترتيب على آلاف الملفات** لم تُقس: العينة 24 ملفًا فقط، وهي كافية لإثبات التخطيط السريع لا للحكم على 5000 ملف.

## ملاحظات خارج الخطة (للاقتراح فقط، لم يُنفَّذ منها شيء)
1. **مجلد `array/target` حجمه 2.4 GB** من هذا البناء. يمكن حذفه بأمان لاستعادة المساحة (`rm -rf array/target`)،
   ويُعاد إنشاؤه بأمر بناء واحد. لم أحذفه لأنك لم تطلبه في هذه المرحلة.
2. `models/` (38 MB) و`tract-onnx` ستزول بالمرحلة 1-أ، وهذا سيقلّص EXE والبناء كثيرًا.
3. التصادم الحالي يُنتج أسماء مقلوبة (`photo` ↔ `photo (1)`) — أرى أن يُعالَج داخل المرحلة 2 لأنه من صميم التسمية، لا في مرحلة مستقلة.
4. `.gitattributes` يحتوي `* text=auto` فيظهر تحذير CRLF عند كل تعديل على `organizer.rs`؛ لا أثر وظيفي، لكنه يلوّث الناتج.

## طريقة التراجع
- الفرع: `plan/phase-0`
- رقم الـ commit: آخر commit في الفرع — `git log -1 --format=%h plan/phase-0`
- التراجع: `git revert $(git log -1 --format=%h plan/phase-0)` ثم `git checkout main` إن أردت العودة كليًا؛ لم يُدمج شيء في `main` ولم يُدفع (`push`) شيء.
