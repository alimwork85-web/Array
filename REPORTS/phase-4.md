# تقرير المرحلة 4 — الواجهة النهائية

الفرع: `plan/phase-4` (متفرّع من `plan/phase-5`).

## ما تغيّر
- `array/src/organizer.rs` — ثمانية مفاتيح جديدة في `Options` (`organize_pictures` … `organize_recordings`، `move_duplicates`) مع `category_enabled`/`set_category`/`set_naming_group`؛ `Plan.categories` و`Plan.naming_groups` (بطاقة لكل فئة ولكل مجموعة: العدد + خمسة أمثلة «قبل ← بعد») و`Plan.left` (ملفات تُركت مع السبب `LeftReason::CategoryOff`)؛ الفئة الموقوفة تبقى ملفاتها في مكانها؛ `DUPLICATES_FOLDER` صار `pub`.
- `array/src/main.rs` — الشاشة الثانية صارت بطاقات ملخص: بطاقة لكل فئة ولكل مجموعة تسمية وللمكررات، فيها العدد وخمسة أمثلة ومفتاح تشغيل وزر «عرض الكل»؛ زر واحد «رتّب»؛ الجدول التفصيلي القديم يظهر خلف «عرض الكل/الملخص»؛ تغيير أي مفتاح يُحفظ ويُعيد حساب الخطة فورًا قبل أي نقل؛ شاشة النتيجة أضافت ثلاثة أرقام (نُقل / أُعيد تسميته / إلى المكررات) وسطر «ما تُرك ولماذا»، مع التراجع وفتح المجلد في الشاشة نفسها.

## الاختبارات
**101 ناجح، 0 فاشل، 1 متجاهَل**. أُضيفت ثلاثة:
- `a_disabled_category_leaves_its_files_alone_with_a_reason` — إيقاف فئة يُبقي ملفاتها ويسجّلها في `left`.
- `the_summary_has_a_count_and_at_most_five_examples_per_card` — بطاقة الصور بعددها وخمسة أمثلة فقط، ومسار الوجهة يبدأ بالمجلد العربي؛ وبطاقة «أسماء الكاميرا» بعددها.
- `duplicates_stay_put_when_the_switch_is_off` — إيقاف مفتاح المكررات يلغي مجلد المراجعة.

آخر 15 سطرًا من `cargo test`:

```
test safety::tests::detects_program_and_project_folders ... ok
test organizer::tests::organizes_a_folder_end_to_end ... ok
test organizer::tests::three_copies_keep_the_clearest_oldest_name_and_review_the_rest ... ok
test organizer::tests::undo_restores_the_exact_original_tree ... ok
test video::tests::year_comes_from_the_video_metadata ... ok
test organizer::tests::running_twice_changes_nothing ... ok
test organizer::tests::reuses_existing_folders_by_arabic_or_english_name ... ok
test organizer::tests::undo_reads_the_folder_journal_without_any_appdata_log ... ok
test organizer::tests::videos_go_to_their_year_and_live_photos_stay_with_their_photo ... ok
test organizer::tests::the_summary_has_a_count_and_at_most_five_examples_per_card ... ok
test organizer::tests::a_year_splits_into_months_only_after_500_files ... ok
test safety::tests::hidden_desktop_ini_does_not_mark_a_folder ... ok

test result: ok. 101 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.76s
```

`cargo build --release`: صفر تحذير. حجم EXE: 12,128,256 ← **12,177,408** بايت (+49 KB للبطاقات).

## التحقق اليدوي (لقطات في `REPORTS/phase-4/`)
مجلد `%LOCALAPPDATA%\Temp\array-phase4` بعينات المرحلة 1 (17 ملفًا: صور بـEXIF، واتساب، لقطة شاشة، `IMG_####`، فيديو+`.srt`، `.heic`+`.mov`، تسجيل، أغنية، PDF وDOCX بعنوان، EPUB، `.7z`، `.msi`، `setup.exe`، ومجلد `tools` محمي).

1. **`1-summary-ar-dark.png`** — العربية + الوضع الداكن: 17 نقلًا، بطاقات الصور (5 أمثلة) والفيديو والمستندات والمكررات، المفتاح و«عرض الكل» في كل بطاقة، والمجلد المحمي ظاهر.
2. **`2-summary-en-light.png`** — الإنجليزية + الوضع الفاتح: نفس البطاقات بنصوص إنجليزية، والمسارات العربية سليمة الاتجاه داخل سطر LTR.
3. **`3-result-screen.png`** — بعد التنفيذ: «17 نُقل · 6 أُعيد تسميته · 0 إلى المكررات» مع «تراجع (17 ملفاً)» و«فتح المجلد» و«رتّب مجلداً آخر» في الشاشة نفسها.

أثناء التحقق أُصلحت ثلاثة أمور ظهرت بالعين لا بالاختبار:
- الأمثلة كانت بفواصل `/` فتقلب اتجاه المسار («2026/صور») → صارت بخطوات ويندوز `\` كما في الجدول القديم، فقرأها الاتجاه صحيحًا.
- محاولة عزل النص بـ`U+2068/U+2069` زادت الفوضى → أُزيلت.
- عدّاد «أُعيد تسميته» كان يحسب النقل فقط (0) → صار يحسب كل ملف تغيّر اسمه (6).

## ما لم يُنفَّذ أو لم يُتحقق منه
- **العرض الضيق**: لم يُختبر تصغير النافذة إلى أضيق حد؛ ضغطات النافذة والكتابة في مربعات الحوار رُفضت بانتظار موافقة المستخدم (`BLOCKED: Action timed out`) خلال الجلسة، فأُكمل التحقق بأداة تحقق مؤقتة (متغيّرا بيئة لاختيار المجلد وتشغيل المراحل) **أُزيلت من الكود قبل هذا الـcommit** — تأكدت بإحصاء `ARRAY_VERIFY` في الـEXE = 0.
- البطاقات في الوضع الإنجليزي/الفاتح تظهر بمحاذاة عمودية غير متساوية (البطاقة الأقصر تتوسّط صفّها). عربيّ + داكن (الافتراضي) مصطفّة كما في اللقطة.
- مفاتيح الفئات والمكررات موجودة في بطاقات الملخص فقط، لم تُضف إلى صفحة «الإعدادات» (لم تنص الخطة على ذلك).
- إعدادات المستخدم `%APPDATA%\Array\settings.json` استُرجعت كما كانت بعد تجربة الوضع الفاتح/الإنجليزي.

## ملاحظات خارج الخطة (للاقتراح فقط، لم تُنفَّذ)
1. اسم `IMG 0548.heic` خرج بمسافة بدل الشرطة السفلية — من `best_name`، لا من هذه المرحلة.
2. صفحة النتيجة تعرض الأرقام فقط؛ لو أردت لاحقًا قائمة «ما تُرك» بأسمائه تحتاج حقلًا إضافيًا في `Outcome`.

## طريقة التراجع
- الفرع: `plan/phase-4`
- رقم الـ commit: آخر commit في الفرع — `git log -1 --format=%h plan/phase-4`
- التراجع: `git revert $(git log -1 --format=%h plan/phase-4)`
- لم يُدمج في `main` ولم يُدفع شيء.
