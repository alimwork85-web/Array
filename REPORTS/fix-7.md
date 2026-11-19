# fix-7: استقرار الترقيم في التشغيل المتكرر

الفرع: `plan/fix-7` من `main` عند `4a2c6bb`. commit الكود: `657876b`.

## ما تغيّر
| الملف | التغيير |
|---|---|
| `array/src/organizer.rs` | في `plan_moves`: ملف موجود أصلاً في مجلد وجهته واسمه الحالي `الاسم` أو `الاسم (رقم)` لا يُرقَّم من جديد؛ يُبقى على اسمه بدل أخذ أول رقم حر. |

قبل الإصلاح كانت الحلقة تبدأ دائمًا من الاسم الأساسي وتأخذ أول رقم حر، ولا تنظر في الاسم الحالي للملف. فتنتقل الفراغات في السلسلة خطوة واحدة كل تشغيل، ولا يستقر المجلد أبدًا.

## الاختبارات
- `cargo test`: **128 ناجح، 0 فاشل، 1 متجاهَل** (كان 127؛ الجديد واحد).
- اختبار جديد: `second_run_renames_nothing` — ينشئ خمسة تسجيلات بالاسم نفسه في `تسجيلات/2026` بأرقام غير متتالية (0، 1، 3، 7، 12)، يشغّل التنظيم، ثم يتأكد أن ثلاثة تشغيلات تالية تُنتج صفر عمليات نقل وأن الشجرة لا تتغير.
- **الاختبار لم يفشل قبل الإصلاح**: الشجرة الصغيرة تُغلق فراغاتها في تشغيل واحد فتمرّ حتى بلا إصلاح. أُبقي حارسًا كما هو. العيب الحقيقي أُعيد إنتاجه وأُثبت نفيه على بيانات `D:\Music` (أسفل).

آخر 15 سطرًا من `cargo test`:
```
test organizer::tests::the_summary_has_a_count_and_at_most_five_examples_per_card ... ok
test tests::a_damaged_journal_falls_back_to_the_appdata_log ... ok
test organizer::tests::videos_go_to_their_year_and_live_photos_stay_with_their_photo ... ok
test video::tests::year_comes_from_the_video_metadata ... ok
test tests::a_stale_journal_falls_back_and_keeps_the_appdata_log_until_done ... ok
test organizer::tests::undo_works_after_the_organized_folder_is_moved ... ok
test organizer::tests::duplicates_never_leave_the_review_folder_without_an_original ... ok
test tests::undo_targets_the_moved_folder_not_the_stored_path ... ok
test tests::interface_smoke_test_organize_then_undo ... ok
test organizer::tests::undo_restores_the_tree_from_every_interruption_point ... ok
test organizer::tests::a_year_splits_into_months_only_after_500_files ... ok
test safety::tests::hidden_desktop_ini_does_not_mark_a_folder ... ok

test result: ok. 128 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.91s
```

`cargo build --release`: **0 تحذير** (بعد `touch` للمصادر كي لا يكون البناء تزايديًا).

## التحقق اليدوي على بيانات حقيقية
على نسخة التحقق `%TEMP%\array_music_copy\Music` (615 ملفًا، 5.8GB) عبر اختبار مؤقت يقرأ `ARRAY_MUSIC_COPY` ثم أُزيل (لا أثر له في الكود ولا في الـEXE: `grep -c ARRAY_MUSIC_COPY target/release/Array.exe` = 0):

| | قبل الإصلاح | بعد الإصلاح |
|---|---|---|
| التشغيل الأول | 13 عملية نقل | **0** |
| الثاني | 11 | 0 |
| الثالث | 10 | 0 |
| الثامن | 9 (دورة لا تنتهي) | 0 |

النتيجة: 587 ملفًا في مكانه الصحيح، صفر عمليات نقل، والمجلد يستقر من التشغيل الأول.

## ما لم يُتحقق منه
- لم تُجرَّب شجرة جديدة من الصفر: النسخة كانت في حالة الدورة عند التطبيق.
- الحالة التي يتغيّر فيها مجلد الملف واسمه معًا (اسم صحيح في مجلد خطأ) لم تُختبر خاصة؛ ينتقل الملف ويأخذ اسمه كما هو إن كان صحيحًا في الوجهة الجديدة، وقد يحتاج تشغيلًا ثانيًا للاستقرار.
- الأسماء المتشابهة التي تختلف بحالة الأحرف على مشارف لا تفرّق بينها (FAT) لا تُختبر.

## ملاحظات خارج النطاق (للمراجعة فقط)
- `HANDOFF.md:49` يقول «الاختبارات: 110/0/1» وهو قديم قبل هذه الجلسة (الصحيح 128 الآن).
- الفراغات في الترقيم تبقى كما هي (لا تُضغط): ملف اسمه `(12)` يبقى `(12)`. الاستقرار أهم من الترتيب، وضغطها ممكن لاحقًا إن أزعجت أحدًا.

## التراجع
```
git revert 657876b
```
الفرع `plan/fix-7` يُحذف بعد الدمج بـ`git branch -d`.
