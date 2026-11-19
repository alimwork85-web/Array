# تقرير الإصلاح 1 — حماية المكررات

الفرع: `plan/fix-1` (متفرّع من `main`، `ea10de0`). الـcommit: `158e652`.

## ما تغيّر
- `array/src/organizer.rs` — في `plan_with` لم يبقَ `مكررات للمراجعة` مجلد «تنظيم من صنع المستخدم» في `kept()`؛ ملفاته تُخطَّط كملفات عادية، فلا تبقى فيه على كل تشغيل.
- `array/src/organizer.rs` — `redirect_duplicates` لم يعد يختار ملفًا داخل مجلد المراجعة «أصلًا» أبدًا: يُرتَّب الأصلاء من خارج المجلد فقط. ملف في المراجعة يُعاد إليه فقط إذا كان له توأم خارجه، وإن لم يكن له توأم بقي على وجهته الطبيعية (فئته).
- `array/src/organizer.rs` — `same_content_groups` و`same_bytes` (جديدتان): بعد تساوي البصمة تُقارن الملفات بايتًا ببايت قبل أي نقل، فتصادم البصمة لا يُرسل شيئًا إلى المراجعة خطأً.
- `array/src/organizer.rs` — `plan.duplicates` يعدّ الملفات التي ستُنقل فعلًا إلى المراجعة، ولا يعدّ ملفًا في المراجعة أصلاً (منعًا لتضخيم العدد المعروض).
- لا حذف في أي مسار، وكل العمليات نقل عادي مُسجَّل في البيان فيعيده التراجع.

## الاختبارات
**115 ناجح، 0 فاشل، 1 متجاهَل** (خط الأساس قبل الإصلاح: 112 ناجح، 1 متجاهَل). أُضيفت ثلاثة:

| الاختبار | فشل قبل الإصلاح؟ | النتيجة الآن |
|---|---|---|
| `duplicates_never_leave_the_review_folder_without_an_original` | **لا — لم يُعِد إنتاج المشكلة** | ناجح |
| `a_review_file_without_a_twin_goes_back_to_its_category` | نعم (الملف بقي في المراجعة في الوضع الافتراضي) | ناجح |
| `a_review_file_with_a_twin_stays_in_review_and_is_never_the_original` | نعم (ملف المراجعة الأقدم اختير أصلًا ونُقل إلى `مستندات/`، والأصل الحقيقي أُرسل إلى المراجعة) | ناجح |

آخر 15 سطرًا من `cargo test`:

```
test tests::a_damaged_journal_falls_back_to_the_appdata_log ... ok
test organizer::tests::videos_go_to_their_year_and_live_photos_stay_with_their_photo ... ok
test organizer::tests::undo_restores_the_exact_original_tree ... ok
test organizer::tests::undo_reads_the_folder_journal_without_any_appdata_log ... ok
test video::tests::year_comes_from_the_video_metadata ... ok
test tests::a_stale_journal_falls_back_and_keeps_the_appdata_log_until_done ... ok
test organizer::tests::duplicates_never_leave_the_review_folder_without_an_original ... ok
test tests::undo_targets_the_moved_folder_not_the_stored_path ... ok
test tests::interface_smoke_test_organize_then_undo ... ok
test organizer::tests::undo_restores_the_tree_from_every_interruption_point ... ok
test organizer::tests::a_year_splits_into_months_only_after_500_files ... ok
test safety::tests::hidden_desktop_ini_does_not_mark_a_folder ... ok

test result: ok. 115 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.87s
```

`cargo build --release` بعد `touch src/organizer.rs`: **صفر تحذير**.

## التحقق اليدوي
التحقق جرى بتنفيذ المسار الحقيقي كاملًا (`plan` ← `execute`) على مجلدات مؤقتة تنشئها الاختبارات في `%TEMP%`، أعيد التنظيم فيها مرتين متتاليتين في الوضعين (الافتراضي وإعادة البناء)، ثم قُرئت الشجرة الناتجة. لم أُجرِ تشغيلًا من الواجهة الرسومية: هذا الإصلاح لا يغيّر أي شيء مرئي، والمقصود من التحقق اليدوي في الخطة (نسخة `D:\Music`) يأتي بعد الإصلاح 5.

## ما لم يُنفَّذ أو لم يُتحقق منه
- **مشكلة الأيتام (76 من 81 في `مكررات للمراجعة` بلا توأم) لم تُعَد إنتاجها.** اختبار الإصلاح 1 المطلوب (ثلاث نسخ متطابقة بوقت تعديل واحد، تنظيم مرتين، وضعان) **نجح قبل الإصلاح**، أي أن هذا السيناريو لا يُنتج المشكلة أصلاً. التزامًا بالخطة لا أفترض سببًا: ما أُثبت هو العيب الآخر فقط — اختيار ملف من داخل المراجعة أصلًا، ونزع الأصل الحقيقي من مكانه.
- لم يُشغَّل على نسخة من `D:\Music` (موعده بعد الإصلاح 5).
- الأيتام الموجودة الآن في `D:\Music` لن تُنقل تلقائيًا بلا تشغيل: بعد هذا الإصلاح يُخرجها التشغيل القادم إلى فئتها لأن `kept()` لم يعد يحمي مجلد المراجعة (وهذا ما يثبته الاختبار الثاني في الوضع الافتراضي). لكن ذلك لم يُجرَّب على نسخة من البيانات الحقيقية بعد.

## ملاحظات خارج الخطة (للاقتراح فقط، لم تُنفَّذ)
1. `collect_files` ما زال يفحص مجلد المراجعة؛ لم أستثنِه لأن استثناءه يمنع الإصلاح المطلوب نفسه (معرفة أي ملف في المراجعة له توأم خارجه)، فبقيت المعالجة في `redirect_duplicates`. لو أردت تنظيمًا أسرع على مكتبات كبيرة، الاستثناء + فحص منفصل للمراجعة ممكن لاحقًا.
2. عند إطفاء خيار «نقل المكررات»، ملفات المراجعة الحالية ستخرج إلى فئاتها (لأن `redirect_duplicates` لا تُستدعى حينها). هذا متسق مع معنى الخيار، لكنه تغيير سلوك لم تنصّ عليه الخطة — أبلغ به ولم أعالجه.
3. لم يعد العدد المعروض `plan.duplicates` يشمل ملفات المراجعة القديمة التي تبقى في مكانها؛ العدد يعني الآن «ما سيُنقل فعلًا».

## طريقة التراجع
```bash
git checkout main
git branch -D plan/fix-1        # أو أبقِ الفرع
git revert 158e652
```
