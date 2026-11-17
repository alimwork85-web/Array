# تقرير المرحلة 2 — إعادة التسمية بالثقة

الفرع: `plan/phase-2` (متفرّع من `plan/phase-1a`، لأن ملفات العقد موجودة هناك فقط).
القاعدة المطبَّقة حرفيًا: **إن لم يوجد مصدر موثوق فلا تسمية. لا اسم مختلَق أبدًا** — وتاريخ آخر تعديل للملف أُزيل من كل مسارات التسمية.

## ما تغيّر
- `array/src/naming.rs` — حُذف `generated_name` و`format_duration` (المدة والتاريخ من `modified()`)، وأُضيف: `Kind` و`NamingGroup` (المجموعات الست) و`DateSource` (Exif / Embedded / FromName)، و`naming_group` (كشف المجموعة من الاسم)، و`trusted_date` (EXIF ← تاريخ داخل الفيديو ← تاريخ الاسم، ولا شيء غيرها)، و`join_name` (التاريخ أولًا ثم الوسم)، و`exif_datetime` (DateTimeOriginal من EXIF بما فيه HEIC)، و`document_title` (عنوان PDF من قاموس `/Info` نصًّا أو ست عشريًّا UTF-16، و`docProps/core.xml` لملفات أوفيس، و`meta.xml` لـODF)، و`pdf_title`/`pdf_literal`/`pdf_hex`/`pdf_text`/`zip_entry_text` لقراءتها.
- `array/src/organizer.rs` — `Options` صار فيه مفتاح لكل مجموعة (`rename_junk_names`, `rename_whatsapp_media`, `rename_screenshots`, `rename_camera_names`, `rename_recordings`, `rename_documents_by_title`؛ كلها مفعّلة افتراضيًا، و`serde(default)` يحفظ توافق `settings.json` القديم) ودالة `naming_enabled`؛ و`RawFileInfo` حمل `kind` و`naming_group` و`audio_tag_name` بدل `needs_name`؛ و`trusted_name` بدل بناء الاسم من السياق؛ وحُذف إغلاق `folder_context` (لم يبق له مستخدم بعد إزالة أسماء السياق)؛ ودالتا `naming_kind` و`tag_name`.
- `array/src/normalizer.rs` — `sanitize_filename` صار يرفض أسماء ويندوز المحجوزة (`CON`, `NUL`, `COM1`…) بإضافة `_`، ويقصّ الاسم عند 120 حرفًا؛ مع اختبار للحالتين.
- `array/Cargo.toml` + `Cargo.lock` — أُضيفت اعتمادية واحدة: `zip = "2"` لقراءة `docProps/core.xml` من DOCX/XLSX/PPTX (ملفات ZIP مضغوطة، لا سبيل لقراءتها بلا فكّ DEFLATE).

**الاسم الجديد:** التاريخ أولًا (`2024-05-12 14-30-05`) بلا مدة وبلا كلمة نوع. الوسم يُضاف فقط حين يكون له مصدر: `واتساب`، `لقطة شاشة`، أو رقم الكاميرا الأصلي (`IMG_2353`). الأغاني: `الفنان - العنوان` من الوسوم. المستندات: عنوانها الداخلي. وإن لم يكن في الاسم إلا تاريخ تعديل — الاسم يبقى كما هو.

## الاختبارات
**88 ناجح، 0 فاشل، 1 متجاهَل** (المتجاهَل هو `export_icons` المتعمّد من قبل). كان الرقم 78؛ أُضيف 10 اختبارات وأُدخلت حالتان في اختبار قائم.

الاختبارات الجديدة (منها الخمسة الإلزامية في الخطة):
- `a_file_with_only_a_modified_date_is_not_renamed` (إلزامي 1)
- `a_template_title_is_not_used_as_a_name` + `template_titles_are_rejected` («Microsoft Word - Document1» مرفوض) (إلزامي 2)
- `camera_names_keep_their_original_number` + `exif_date_is_read_from_the_photo` (إلزامي 3)
- `ten_photos_in_the_same_second_get_ten_names` (إلزامي 4)
- `a_disabled_group_keeps_its_files_unnamed` (إلزامي 5)
- `a_document_gets_its_internal_title`، `whatsapp_and_screenshot_files_get_their_label`، `groups_are_recognized_before_anything_else`، `only_a_trusted_date_counts`، `names_are_date_first_without_duration`.

**تعديل توقّعات ثلاثة اختبارات قائمة (بموافقتك):**
1. `meaningless_names_get_a_name_from_their_context` → `meaningless_names_stay_without_a_trusted_source`: كان يؤكد أن الملفات بلا EXIF ولا تاريخ تأخذ اسماً من السياق وتاريخ التعديل؛ الآن يؤكد بقاءها باسمها. (هذا هو نص الخطة حرفيًا، ومعك أُقرّ.)
2. `default_mode_never_touches_existing_folders`: تأكيد واحد تغيّر من `Photo - 20…` إلى بقاء `133649 466985853345771 200500525.jpg`.
3. `durations_are_readable` حُذف مع الدالة `format_duration` التي حُذفت بموجب «تُحذف المدة من الاسم»؛ اختبار دالة محذوفة لا معنى له. بديله `names_are_date_first_without_duration`.
4. تغيّر شكل الاسم أيضًا فرض تحديث توقّع اسمين في `videos_by_year_can_be_turned_off` و`videos_go_to_their_year_and_live_photos_stay_with_their_photo` (`Video - 2019-07-04 10-10` ← `2019-07-04 10-10`، و`VID-20210101-WA0001` ← `2021-01-01 واتساب`).

آخر 15 سطرًا من `cargo test`:

```
test video::tests::year_comes_from_the_name_first ... ok
test storage::tests::user_edits_are_loaded_and_partial_files_get_defaults ... ok
test organizer::tests::reuses_an_existing_artist_folder ... ok
test organizer::tests::never_touches_programs_projects_scripts_or_web_pages ... ok
test storage::tests::broken_settings_file_is_reported_and_left_untouched ... ok
test organizer::tests::reuses_existing_folders_by_arabic_or_english_name ... ok
test safety::tests::selected_folder_is_refused_when_it_is_a_program ... ok
test storage::tests::undo_log_round_trip_and_clear ... ok
test organizer::tests::running_twice_changes_nothing ... ok
test organizer::tests::videos_go_to_their_year_and_live_photos_stay_with_their_photo ... ok
test safety::tests::detects_program_and_project_folders ... ok
test organizer::tests::undo_restores_the_exact_original_tree ... ok
test organizer::tests::organizes_a_folder_end_to_end ... ok
test video::tests::year_comes_from_the_video_metadata ... ok
test safety::tests::hidden_desktop_ini_does_not_mark_a_folder ... ok

test result: ok. 88 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.74s
```

`cargo build --release`: **صفر تحذير**. حجم EXE: 11,224,576 ← **12,023,296** بايت (+798,720؛ ثمن حزمة `zip` وحدها).

## التحقق اليدوي
مجلد مؤقت جديد `%LOCALAPPDATA%\Temp\array-phase2` فيه 10 ملفات حقيقية: صورة EXIF، صورة بلا EXIF، صورة واتساب (اسمها فيه تاريخ وفيها EXIF بتاريخ آخر لاختبار الأسبقية)، لقطة شاشة، PDF بعنوان داخلي حقيقي، PDF بعنوان قالب، DOCX بـ`docProps/core.xml`، فيديو واتساب، وMOV بلا تاريخ. شُغّل `target/release/Array.exe` فعليًا، واختير المجلد من نافذة «تحديد مجلد»، ثم «معاينة التغييرات» ثم «رتّب».

لقطة المعاينة: `REPORTS/phase-2/1-preview-new-names.png`، واختيار المجلد: `REPORTS/phase-2/2-folder-selected.png`.

النتيجة على القرص بعد التنفيذ:
```
Documents/8877aa.pdf                          ← عنوان القالب مرفوض: الاسم بقي
Documents/Annual Report 2024.pdf              ← العنوان الداخلي
Documents/خطة المشروع.docx                     ← docProps/core.xml
Pictures/2024-05-12 14-30 لقطة شاشة.png
Pictures/2024-05-12 14-30-05 IMG_2353.jpg      ← EXIF + رقم الكاميرا
Pictures/2024-05-12 14-30-05.jpg               ← EXIF، اسم رديء
Pictures/2025-01-01 10-00-00 واتساب.jpg        ← EXIF تغلّب على تاريخ الاسم (واتساب)
Pictures/4749d951.jpg                          ← لا EXIF ولا تاريخ في الاسم: بقي باسمه
Videos/2024/2024-05-12 واتساب.mp4
Videos/IMG 9999.mov                            ← بلا تاريخ: بقي (مجلد السنة لم يُنشأ)
```
ثم ضُغط «تراجع (10 ملفات)» وتأكيد النافذة، فرجعت الملفات العشرة إلى أسمائها وأماكنها الأصلية **حرفيًا** (تحقق بالقراءة من القرص بعد التراجع)، وحُذفت المجلدات التي أنشأها التنظيم.

## ما لم يُنفَّذ أو لم يُتحقق منه، ولماذا
- **مفاتيح المجموعات الست في الواجهة لم تُضَف** — الخطة تقول «في `Options` الآن» والواجهة في المرحلة 4. الاختبار يثبت عمل المفتاح من `Options`.
- **لم أجرّب HEIC ولا M4A حقيقيًا** (لا عيّنة عندي)؛ الحزمة `kamadak-exif` 0.5.5 تقرأ EXIF من HEIF/HEIC، ووسوم الصوت تُقرأ بـ`lofty` القائم. الملفان المعدومان في الاختبارات لا يغطيان هذين المسارين.
- **الفيديو بلا وقت في الاسم يأخذ اليوم فقط**: صيغة واتساب `VID-YYYYMMDD-WA####` لا تحمل وقتًا، فلا يمكن كتابة `HH-MM-SS` بلا اختلاق؛ الاسم الناتج `2024-05-12 واتساب.mp4`.
- **ملف صوتي بلا وسوم وبلا تاريخ في الاسم لا يُسمّى** (مذكرات صوتية باسم `IMG_0001.m4a` مثلًا) — نتيجة مباشرة للقاعدة الذهبية؛ لو أردت تسميتها بتاريخ التعديل فهذا نقض للقاعدة ويحتاج قرارك.

## ملاحظات خارج الخطة (للاقتراح فقط، لم تُنفَّذ)
1. **نص مفتاح «تحسين أسماء الملفات» في الواجهة صار قديمًا**: يقول حرفيًا «…ويعطي الملفات مثل 4749d951.mp3 أسماء من سياقها»، وهذا لم يعد صحيحًا بعد المرحلة 2. النص في `main.rs` ويحتاج تحديثًا في المرحلة 4.
2. `%APPDATA%\Array\face_cache.json` (16.2 MB من نسخة الوجوه) حُذف بطلب صريح من صاحب المشروع؛ لم يبقَ في مجلد بيانات البرنامج إلا `settings.json`.
3. `sanitize_filename` يُستخدم لأسماء المجلدات أيضًا، فصار يقصّ أسماء المجلدات عند 120 حرفًا — سلوك محافظ ومقصود، بلا أثر على الاختبارات.
4. قراءة عنوان PDF تفحص أول ميجابايت وآخر ميجابايت فقط (سرعة)؛ PDF ضخم فيه `/Title` في المنتصف فقط لن يُقرأ — نادر، وتكلفته مقبولة.
5. `zip` زادت حجم EXE بـ800 كيلوبايت؛ لو أردت لاحقًا تقليصًا فيمكن جعلها اختيارية بـfeature، لكن هذا تعقيد بلا مقابل الآن.

## طريقة التراجع
- الفرع: `plan/phase-2`
- رقم الـ commit: آخر commit في الفرع — `git log -1 --format=%h plan/phase-2`
- التراجع: `git revert $(git log -1 --format=%h plan/phase-2)` (يعيد `naming.rs` و`organizer.rs` و`normalizer.rs` و`Cargo.toml`/`Cargo.lock` إلى حالة المرحلة 1-أ).
- لم يُدمج في `main` ولم يُدفع شيء.
