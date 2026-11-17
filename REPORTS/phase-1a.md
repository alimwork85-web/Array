# تقرير المرحلة 1-أ — إزالة التعرّف على الوجوه

الفرع: `plan/phase-1a` (متفرّع من `plan/phase-0`، لأن ملفات العقد `AGENTS.md`/`PLAN.md`/`DIAGNOSIS.md` موجودة هناك فقط).

## ما تغيّر
- `array/src/faces.rs` — **حُذف** (493 سطرًا: النماذج، YuNet/SFace، ذاكرة البصمات).
- `array/models/` — **حُذف بالكامل** (38.2 MB: نموذجا onnx + README).
- `array/Cargo.toml` — حُذف `tract-onnx` و`rayon` (كانا مستخدمين في ملف الوجوه فقط؛ `image` بقي لأنه يحمّل الأيقونة، و`kamadak-exif` بقي لأنه للمرحلة 2).
- `array/Cargo.lock` — حُذفت شجرة الاعتماديات الخاصة بهما (623 سطرًا).
- `array/src/organizer.rs` — حُذف `FaceSorting` و`sort_by_faces` و`face_readable` و`spread` و`sort_by_faces` من `Options` وحقول `face_matches`/`people`/`face_note` من `Plan`، ومعامل `face_cache` من `plan_with`، و`PlanProgress` كاملة (لم تبقَ لها مرحلة تُبلّغ عنها).
- `array/src/main.rs` — حُذف `mod faces`، ومفتاح «رتّب الصور بالوجوه» من الإعدادات، وبطاقة «عُرفت بالوجه» وعددها من شاشة المعاينة، وسطر «الأشخاص الذين تعلّم البرنامج وجوههم»، وزر «مسح ذاكرة الوجوه» ونافذته، وسطر «نماذج الوجوه» في «حول»، وعمود درجة الوجه من جدول المعاينة، ومعامل التقدّم من مهمة التخطيط.
- `array/src/storage.rs` — حُذف `face_cache_path` و`face_cache_size` و`clear_face_cache`.
- `array/src/icons.rs` — حُذفت أيقونتا `Person` و`Trash` (كانتا لهذه الشاشات فقط) فصار عدد الأيقونات 23 بدل 25.
- `array/src/i18n.rs` — أُبقي `Noun::Photo` (اختباراته ما زالت قائمة) مع `#[allow(dead_code)]` وسبب مكتوب؛ لا مستخدم إنتاجي له حتى تُضاف مجلدات السنوات.
- `README.md` — أُزيل ذكر الوجوه من العربي والإنجليزي: سطر الخصوصية، وصف الميزة «الصور حسب الأشخاص»، ذاكرة الوجوه في مسار البيانات، وسؤالا الشائعة عن الوجوه (استُبدلا بسؤال «هل يتصل بالإنترنت؟»)، وسطر ترخيص النماذج (كان يشير إلى `array/models/README.md` المحذوف)، وحُدّث حجم الملف من 64 MB إلى 11 MB.

## الاختبارات
**78 ناجح، 0 فاشل، 1 متجاهَل** (المتجاهَل هو مُصدِّر الأيقونات المتعمّد `export_icons`).
الأربعة التي نقصت عن 82 هي اختبارات `faces.rs` نفسه التي حُذف بموجب الخطة حرفيًا:
`year_generic_and_private_folders_are_not_people`، `models_load_and_blank_image_has_no_face`،
`alignment_maps_template_points_to_themselves`، `unreadable_images_are_reported_not_crashed`.
لم يُحذف ولم يُعدَّل أي اختبار آخر، ولا شيء من `organizer`/`safety`/`storage`/`video`/`grouping`.

آخر 15 سطرًا من `cargo test`:

```
test organizer::tests::meaningless_names_get_a_name_from_their_context ... ok
test safety::tests::selected_folder_is_refused_when_it_is_a_program ... ok
test organizer::tests::reuses_existing_folders_by_arabic_or_english_name ... ok
test organizer::tests::never_touches_programs_projects_scripts_or_web_pages ... ok
test storage::tests::undo_log_round_trip_and_clear ... ok
test video::tests::year_comes_from_the_video_metadata ... ok
test organizer::tests::running_twice_changes_nothing ... ok
test safety::tests::detects_program_and_project_folders ... ok
test organizer::tests::videos_go_to_their_year_and_live_photos_stay_with_their_photo ... ok
test organizer::tests::organizes_a_folder_end_to_end ... ok
test organizer::tests::undo_restores_the_exact_original_tree ... ok
test safety::tests::hidden_desktop_ini_does_not_mark_a_folder ... ok

test result: ok. 78 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.72s
```

`cargo build --release`: ناجح في 7.84 ثانية، **صفر تحذير** (التحذيرات الأربعة التي ظهرت بعد الحذف كانت كلها بقايا الوجوه — `PlanProgress`، `Noun::Photo`، `Icon::Person`، `Icon::Trash` — وعولجت بحذف ما يجب حذفه و`allow` واحد مبرَّر، لا بتعطيل تحذير عام).

## حجم EXE قبل وبعد
| | بايت | ميجابايت |
|---|---|---|
| قبل الإزالة | 67,712,512 | 64.6 |
| بعد الإزالة | 11,224,576 | 10.7 |

**الفرق: −56,487,936 بايت (−83.4%).**

## التحقق اليدوي
شُغّلت النسخة الجديدة (`target/release/Array.exe` بلا مجلد `models` — أي أنها تعمل مستقلة) على مجلدين مؤقتين:
1. `array-diag` (فيه `setup_tool.exe` في الجذر): رُفض المجلد برسالة واضحة ولم يتحرك ملف — نفس سلوك ما قبل الحذف، بلا انهيار (`REPORTS/phase-1a/2-refused-folder.png`).
2. `array-diag2` (بلا exe في الجذر): ظهرت شاشة المعاينة بـ22 عملية و3 بطاقات إحصاء فقط (بلا بطاقة «عُرفت بالوجه»)، وجدول المعاينة بلا عمود درجة وجه (`REPORTS/phase-1a/3-preview-after-removal.png`).
3. شاشة البداية بلا مفتاح الوجوه (`REPORTS/phase-1a/1-no-face-toggle.png`).
لم يُنفَّذ الترتيب نفسه في هذا التشغيل (المعاينة كافية لاختبار مسار التخطيط)؛ الترتيب والتراجع مُختبَران في `cargo test` ولم يتغيّر منهما سطر.

## ما لم يُنفَّذ أو تعارض في الخطة
**تعارض واحد يستحق قرارك:** تقول الخطة في 1-أ: «سلوك الصور بعد الإزالة: تذهب إلى `صور/السنة`». لكن مجلدات السنوات للصور هي بند من **المرحلة 1** («الصور والفيديو: `السنة/`»). بعد إزالة الوجوه الآن الصور تذهب إلى `صور/` **مسطحة بلا سنة** (كما في لقطة المعاينة)، ولم أُنفّذ مجلدات السنوات هنا حتى لا أنفّذ جزءًا من المرحلة 1 قبل أوانه. إن أردت أن تكون جملة 1-أ حرفية الآن قل «أضف السنة للصور» وأنفّذها كخطوة صغيرة منفصلة.

## ملاحظات خارج الخطة (للاقتراح فقط)
1. **صور `docs/screenshots/` في README** قد تكون ملتقطة قبل الحذف، فربما يُظهر إحداها مفتاح الوجوه أو بطاقة «عُرفت بالوجه». لم أعد التقاطها (ليست في نطاق المرحلة، وREADME لا يعرض ميزة الوجوه الآن).
2. **`array/target` = 2.4 GB** من البناء الأول؛ حذفه آمن ويستعيد المساحة.
3. `Cargo.toml` ما زال يحمل `kamadak-exif` بلا مستخدم حتى المرحلة 2 — سيبقى بلا تحذير لأن Cargo لا يحذّر من اعتمادية غير مستخدمة، وذُكر هنا حتى لا يُنسى أنه غير مستخدم الآن.
4. تحذير CRLF من `.gitattributes` يظهر عند كل تعديل؛ بلا أثر وظيفي.

## طريقة التراجع
- الفرع: `plan/phase-1a`
- رقم الـ commit: آخر commit في الفرع — `git log -1 --format=%h plan/phase-1a`
- التراجع: `git revert $(git log -1 --format=%h plan/phase-1a)` (يعيد `faces.rs` و`models/` وكل ما حُذف، لأن الحذف داخل الـ commit).
- لم يُدمج في `main` ولم يُدفع شيء.
