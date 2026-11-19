# naming-golden — المجموعة الذهبية للأسماء (88 اسمًا)

**الفرع:** `plan/naming-golden` من `main` (‏`52fd138`)
**الـ commit:** `c1c9b6a` — `Test: lock the 88-name golden naming set as it is now`
**الحالة:** اختبارات فقط. لم يُعدَّل أي سطر من منطق التسمية. لا دمج ولا push.

## ما تغيّر

| الملف | التغيير |
|---|---|
| `array/src/organizer.rs` | +139 سطرًا في نهاية `mod tests`: جدول `GOLDEN_NAMES` (88 صفًا) واختبار `golden_naming_set_is_unchanged`. صفر حذف، وصفر تعديل على كود الإنتاج. |

الاختبار لكل صف يثبّت ثلاثة أشياء على الاسم المنظَّف كما يفعل `plan_with`:
1. `naming::is_junk_name` — هل يعتبره اسمًا رديئًا.
2. `naming::naming_group` — المجموعة (أو `None` = الاسم يبقى).
3. `OrganizerEngine::trusted_name` — الاسم الجديد النهائي، أو `None` بلا إعادة تسمية.

**لماذا هنا وليس `array/tests/naming_golden.rs`:** الحزمة برنامج تنفيذي (`[[bin]]`) بلا `lib`، و`trusted_name` خاص، فاختبار تكاملي في `tests/` لا يستطيع الوصول إلى الوحدات. فتح مكتبة أو نقل الوحدات كان سيعني إعادة هيكلة، وهي ممنوعة. فالاختبار أُضيف بجانب اختبارات الطبقة نفسها في `organizer.rs`، بلا تغيير في أي مسار.

## الاختبارات

| ما قيس | النتيجة |
|---|---|
| `organizer::tests::golden_naming_set_is_unchanged` | ناجح — 88/88 صفًا |
| المجموع | **132 ناجح / 0 فاشل / 1 متجاهَل** (كان 131 + 1 متجاهَل) |
| `cargo build --release` | نظيف، صفر تحذير |

آخر 15 سطرًا من `cargo test`:

```
test organizer::tests::videos_go_to_their_year_and_live_photos_stay_with_their_photo ... ok
test organizer::tests::undo_restores_the_exact_original_tree ... ok
test video::tests::year_comes_from_the_video_metadata ... ok
test tests::a_stale_journal_falls_back_and_keeps_the_appdata_log_until_done ... ok
test organizer::tests::duplicates_never_leave_the_review_folder_without_an_original ... ok
test tests::trash_job_reports_when_nothing_is_movable ... ok
test tests::review_scan_runs_off_the_ui_thread ... ok
test tests::undo_targets_the_moved_folder_not_the_stored_path ... ok
test tests::interface_smoke_test_organize_then_undo ... ok
test organizer::tests::undo_restores_the_tree_from_every_interruption_point ... ok
test organizer::tests::a_year_splits_into_months_only_after_500_files ... ok
test safety::tests::hidden_desktop_ini_does_not_mark_a_folder ... ok

test result: ok. 132 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.86s
```

**هل الاختبار يفشل فعلًا عند تغيّر السلوك؟** نعم، أُثبت بمطفأة (mutation) مؤقتة: تعطيل نمط مسجّل الصوت `is_audio_recording` في `naming.rs` أسقط الاختبار (`0 passed; 1 failed`)، ثم أُعيد الملف من نسخته، و`git status` نظيف. لم يبقَ أثر للمطفأة.

## التحقق من القائمة

- الأسماء في ملف الاختبار استُخرجت آليًا وقورنت بقائمة `Array-naming-golden-قائمة-مقترحة.md` المعتمدة: **88 = 88، و`diff` فارغ**. لا اسم غير مموَّه، ولا اسم من `D:\Music` الحقيقي.
- لم يُقرأ أي ملف من مكتبة علي في هذه الخطوة: الأسماء نصوص، والاختبار لا يفتح أي ملف من القرص (`trusted_date` يفشل بهدوء بلا ملف فيُرجع تاريخ الاسم، وهو المسار المقصود).

## ما لم يُتحقق منه، ولماذا

- **طبقات ما بعد الاسم خارج المجموعة:** وسوم lofty، وEXIF، وعنوان PDF/DOCX الداخلي، وتاريخ داخلي للفيديو — كلها تحتاج ملفات حقيقية ولا تُعاد إنتاجها بالاسم وحده. تبقى مغطاة بالاختبارات الحالية التي تُنشئ ملفات (`jpeg_with_date`، `pdf_with_title`، اختبارات الفيديو)، ولا تدخل هذه المجموعة.
- **ملف `%TEMP%\golden_candidates.tsv`** (الأسماء الأصلية مع نواتجها) بقي محليًا كما هو ولم يُرفع.
- **26 صفًا من أصل 88** مصدرها أنماط الاختبارات الحالية نفسها، لا مكتبة علي: `D:\Music` لا يحتوي صورًا ولا مستندات ولا واتساب، وبلا هذه الأنماط تبقى المجموعة ناقصة.

## ملاحظات خارج النطاق — مسجَّلة كما هي، ولم تُصلح

1. **تسجيل صوتي بالعربية لا يُعامل كتسجيل:** `تسجيل صوتي - 2026-09-12 23-35 - 1h10m05s` يبقى اسمه، لأن نمط مسجّل الصوت يقبل البادئات الإنجليزية فقط (`Audio`/`Recording`/`Voice`/`AI Music`)، بينما `2026-09-12 23-26 تسجيل صوتي` (بلا شرطات ولا مدة) يُعاد تسميته. سلوك غير متسق، مثبَّت الآن في المجموعة (الصف 36) حتى لا يتغيّر صامتًا.
2. **`AlbumArtSmall.jpg` و`Folder.jpg`:** الأول يُصنَّف «اسمًا رديئًا» (المجموعة `Junk`) ثم لا يُعاد تسميته لغياب تاريخ موثوق، والثاني لا يُصنَّف رديئًا أصلًا. الفرق مثبَّت في الصفين 1–2 و6.
3. **`20250606 162959 IMG 2054`:** يُقرأ منه التاريخ (2025-06-06 16-29) لكنه ليس في مجموعة الكاميرا، لأن رقم الكاميرا في وسط الاسم لا يطابق `^IMG`، فيبقى الاسم. مثبَّت في الصف 34.

ملاحظة إضافية عن المجموعة نفسها: هي تقيس **الناتج الحالي بصدق**، فلو كان في أحد الصفوف حكم يبدو خاطئًا فهو الآن موثَّق لا مُصلَح. تعديل أي توقع يحتاج قرارًا صريحًا من علي.

## التراجع

```bash
git revert c1c9b6a
```
الفرع كامل يُحذف بلا أثر على `main`: `git branch -D plan/naming-golden`.
