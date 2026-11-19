# تقرير الإصلاح 2 — مجلدات البرنامج ليست مجلدات المستخدم

الفرع: `plan/fix-2` (متفرّع من `plan/fix-1`، لأن الإصلاحين يلمسان نفس الدالة `kept()`).
الـcommit: `f9cc8b9`.

## ما تغيّر
- `array/src/organizer.rs` — قبل `kept()` تُجمع مسارات مجلدات البرنامج الثلاثة (`misc_dir` و`ai_dir` ومجلد فئة التسجيلات) **فقط إذا كان `.array` موجودًا في الجذر المختار**، ثم تُستثنى ملفاتها من «المحفوظ». تُقارن المسارات نفسها لا الأسماء، فلا يلتبس على البرنامج مجلد مستخدم يحمل الاسم نفسه في موضع آخر.
- الملف الذي اسم فئته هو نفسه اسم الجذر (مثل `D:\Music` نفسه) لا يُضاف إلى القائمة، وإلا صار كل ملف في الجذر غير محفوظ.
- بلا `.array` تبقى القائمة فارغة: أي مجلد بهذه الأسماء أنشأه المستخدم ولا يُلمس.
- لم يُعدَّل اختبار `running_twice_changes_nothing` ولا ضُعِّف، وما زال ناجحًا.

## الاختبارات
**117 ناجح، 0 فاشل، 1 متجاهَل** (قبل الإصلاح: 115). أُضيف اثنان:

| الاختبار | فشل قبل الإصلاح؟ | النتيجة |
|---|---|---|
| `a_misc_folder_made_by_the_program_is_not_a_user_folder` | نعم — الملفات الثلاثة بقيت في `منوعات` | ناجح |
| `a_misc_folder_without_a_journal_belongs_to_the_user` | لا (اختبار حماية: لازم يبقى ناجحًا قبل وبعد) | ناجح |

أُعيد التحقق من اللون الأحمر بعد كتابة الكود: عُطِّل سطر `!program_dirs…` مؤقتًا فسقط الاختبار الأول ونجح الثاني، ثم أُعيد السطر.

آخر 15 سطرًا من `cargo test`:

```
test organizer::tests::undo_works_after_the_organized_folder_is_moved ... ok
test organizer::tests::the_summary_has_a_count_and_at_most_five_examples_per_card ... ok
test tests::a_damaged_journal_falls_back_to_the_appdata_log ... ok
test video::tests::year_comes_from_the_video_metadata ... ok
test organizer::tests::videos_go_to_their_year_and_live_photos_stay_with_their_photo ... ok
test tests::a_stale_journal_falls_back_and_keeps_the_appdata_log_until_done ... ok
test organizer::tests::duplicates_never_leave_the_review_folder_without_an_original ... ok
test tests::undo_targets_the_moved_folder_not_the_stored_path ... ok
test tests::interface_smoke_test_organize_then_undo ... ok
test organizer::tests::undo_restores_the_tree_from_every_interruption_point ... ok
test organizer::tests::a_year_splits_into_months_only_after_500_files ... ok
test safety::tests::hidden_desktop_ini_does_not_mark_a_folder ... ok

test result: ok. 117 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.86s
```

`cargo build --release` بعد `touch src/organizer.rs`: **صفر تحذير**.

## التحقق اليدوي
جُرِّب على مجلدين مؤقتين في `%TEMP%` اسم كل منهما `موسيقى` (ليكون هو مجلد الصوتيات نفسه كما في `D:\Music`)، وثلاثة ملفات لفنان واحد داخل `منوعات`:
- مع `.array`: خرجت الثلاثة إلى `موسيقى\عمرو دياب\` وخلت `منوعات` (المسار الحقيقي `plan` ← `execute`).
- بلا `.array`: بقيت الثلاثة في `منوعات` كما هي.

لم أُجرِ تشغيلًا من الواجهة الرسومية: لا تغيير مرئي في هذا الإصلاح.

## ما لم يُنفَّذ أو لم يُتحقق منه
- لم يُجرَّب على نسخة من `D:\Music` (موعده بعد الإصلاح 5).
- مجلد التسجيلات داخل القائمة، لكن لا اختبار مخصّص له: الخطة طلبت اختبارين فقط، والمسار مشترك مع المنوعات. يُتحقق منه في التحقق الشامل.
- الحالة التي يكون فيها اسم الجذر نفسه اسم فئة تسجيلات غير مختبَرة (استُثنيت بحارس `d != source_path`).

## ملاحظات خارج الخطة (للاقتراح فقط، لم تُنفَّذ)
1. في التشغيل **الأول** على مجلد لم ينظّمه البرنامج قبلًا، ما زال `منوعات` (وغيره) محفوظًا إن كان موجودًا — لأن `.array` لا يكون موجودًا بعد. أي أن التصحيح يبدأ من التشغيل الثاني. هذا نصّ الخطة حرفيًا، لكنه يعني أن خطأ التشغيل الأول على مجلد «منوعات» قديم لن يُصحَّح.
2. لو أردت لاحقًا تمييزًا أدق: يمكن اعتبار المجلد مجلد برنامج إذا كان **فارغًا** أو يحتوي ملفات نظّمها البرنامج (بمسارات مسجّلة في البيان)، بدل الاعتماد على وجود `.array`.

## طريقة التراجع
```bash
git revert f9cc8b9
```
