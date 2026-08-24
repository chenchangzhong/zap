use std::collections::HashMap;

use rangemap::RangeMap;
use unindent::Unindent as _;
use warp_editor::multiline::{MultilineStr, MultilineString};

use crate::code::editor::diff::ChangeType;

use super::DiffModel;
use crate::code::editor::DiffStatus;

#[test]
fn test_diff_generation() {
    use warpui::App;
    App::test((), |_| async move {
        let (change_mapping, deletion_mapping) = DiffModel::compute_diff_internal(
            MultilineStr::try_new("Hello World\nThis is the second line.\nThis is the third.")
                .unwrap(),
            MultilineStr::try_new(
                "Hallo Welt\nThis is the second line.\nThis is life.\nMoar and more",
            )
            .unwrap(),
        )
        .await;
        assert_eq!(
            change_mapping,
            RangeMap::from_iter([
                (
                    0..1,
                    ChangeType::Replacement {
                        replaced_range: 0..1,
                        insertion: vec![0..5, 6..10],
                        deletion: vec![0..5, 6..11]
                    }
                ),
                (
                    2..4,
                    ChangeType::Replacement {
                        replaced_range: 2..3,
                        insertion: vec![8..13, 14..22, 23..27,],
                        deletion: vec![8..11, 12..18]
                    }
                )
            ])
        );
        assert!(deletion_mapping.is_empty());

        let (change_mapping, deletion_mapping) = DiffModel::compute_diff_internal(
            MultilineStr::try_new("Hello World\nThis is the second line.\nThis is the third.")
                .unwrap(),
            MultilineStr::try_new("Hello World\nThis is the third.").unwrap(),
        )
        .await;
        assert!(change_mapping.is_empty());
        assert_eq!(deletion_mapping, HashMap::from([(1, 1..2)]));

        let (change_mapping, deletion_mapping) = DiffModel::compute_diff_internal(
            MultilineStr::try_new("Hello\nWorld\n").unwrap(),
            MultilineStr::try_new("Hallo\nWorlds\n").unwrap(),
        )
        .await;

        assert_eq!(
            change_mapping,
            RangeMap::from_iter([(
                0..2,
                ChangeType::Replacement {
                    replaced_range: 0..2,
                    insertion: vec![0..5, 6..12],
                    deletion: vec![0..5, 6..11]
                }
            ),])
        );
        assert!(deletion_mapping.is_empty());
    });
}

#[test]
fn test_reverse_action() {
    use warpui::App;
    App::test((), |_| async move {
        let mut diff_model = DiffModel::new();
        diff_model.set_base(MultilineString::apply(
            "Hello World\nThis is the second line\n",
        ));
        diff_model
            .compute_diff_for_test("Hallo World\nThis is the second line\nNew".to_string())
            .await;

        assert_eq!(diff_model.diff_hunk_count(), 2);
        assert_eq!(
            diff_model.reverse_action_by_diff_hunk_index(0),
            Some((0..1, "Hello World\n".to_string()))
        );

        assert_eq!(
            diff_model.reverse_action_by_diff_hunk_index(1),
            Some((2..3, "".to_string()))
        );

        diff_model
            .compute_diff_for_test("Hello World\n".to_string())
            .await;
        assert_eq!(
            diff_model.reverse_action_by_diff_hunk_index(0),
            Some((1..1, "This is the second line\n".to_string()))
        );
    });
}

#[test]
fn test_reverse_action_replaced_newlines() {
    use warpui::App;
    App::test((), |_| async move {
        let mut diff_model = DiffModel::new();
        let base_text = r"

            abc
            def

            ghi

            jkl
            mno


            pqr

            stu

        "
        .unindent();
        diff_model.set_base(MultilineString::apply(&base_text));

        // Replace with text:
        // * Leading newline before "abc"
        // * "def" and the following line
        // * Newline between "def" and "ghi"
        // * "pqr" and the preceding line
        // * Trailing newline at end of file
        let modified_text = r"
            replaced first empty line
            abc
            changed def
            changed line after def
            ghi
            changed line between ghi and jkl
            jkl
            mno

            changed line before pqr
            changed pqr

            stu
            replaced last empty line
        "
        .unindent();
        diff_model.compute_diff_for_test(modified_text).await;

        assert_eq!(diff_model.diff_hunk_count(), 5);

        // Reversing the leading newline change
        assert_eq!(
            diff_model.reverse_action_by_diff_hunk_index(0),
            Some((0..1, "\n".to_string()))
        );

        // Reversing "def" and following newline
        assert_eq!(
            diff_model.reverse_action_by_diff_hunk_index(1),
            Some((2..4, "def\n\n".to_string()))
        );

        // Reversing the line changed between "ghi" and "jkl"
        assert_eq!(
            diff_model.reverse_action_by_diff_hunk_index(2),
            Some((5..6, "\n".to_string()))
        );

        // Reversing changing "pqr" and the line before it
        assert_eq!(
            diff_model.reverse_action_by_diff_hunk_index(3),
            Some((9..11, "\npqr\n".to_string()))
        );

        // Reversing changing the last line
        assert_eq!(
            diff_model.reverse_action_by_diff_hunk_index(4),
            Some((13..14, "\n".to_string()))
        );
    });
}

#[test]
fn test_reverse_action_replaced_text() {
    use warpui::App;
    App::test((), |_| async move {
        let mut diff_model = DiffModel::new();
        let base_text = r"
            abc
            def
            ghi

            jkl
            mno
            pqr

            stu
            vwx
            yz
        "
        .unindent();
        diff_model.set_base(MultilineString::apply(&base_text));

        // Replace with a newline:
        // * First line "abc"
        // * "ghi", which is followed by a newline
        // * "mno"
        // * "stu", which is preceded by a newline
        // * Last line "yz"
        let modified_text = r"

            def


            jkl

            pqr


            vwx

        "
        .unindent();
        diff_model.compute_diff_for_test(modified_text).await;

        assert_eq!(diff_model.diff_hunk_count(), 5);

        // Reversing the leading newline change
        assert_eq!(
            diff_model.reverse_action_by_diff_hunk_index(0),
            Some((0..1, "abc\n".to_string()))
        );

        // Reversing "ghi"
        assert_eq!(
            diff_model.reverse_action_by_diff_hunk_index(1),
            Some((3..4, "ghi\n".to_string()))
        );

        // Reversing "mno"
        assert_eq!(
            diff_model.reverse_action_by_diff_hunk_index(2),
            Some((5..6, "mno\n".to_string()))
        );

        // Reversing "stu"
        assert_eq!(
            diff_model.reverse_action_by_diff_hunk_index(3),
            Some((8..9, "stu\n".to_string()))
        );

        // Reversing changing the last line
        assert_eq!(
            diff_model.reverse_action_by_diff_hunk_index(4),
            Some((10..11, "yz\n".to_string()))
        );
    });
}

#[test]
fn test_reverse_action_deleted_lines() {
    use warpui::App;
    App::test((), |_| async move {
        let mut diff_model = DiffModel::new();
        let base_text = r"

            abc
            def

            ghi

            jkl
            mno


            pqr


            stu
            vwx

            yz

        "
        .unindent();
        diff_model.set_base(MultilineString::apply(&base_text));

        // Delete:
        // * Leading newline before "abc"
        // * Newline between "def" and "ghi"
        // * "mno" followed by a newline
        // * newline followed by "stu"
        // * Trailing newline after "yz"
        let modified_text = r"
            abc
            def
            ghi

            jkl

            pqr

            vwx

            yz
        "
        .unindent();
        diff_model.compute_diff_for_test(modified_text).await;

        assert_eq!(diff_model.diff_hunk_count(), 5);

        // Reversing the leading newline deletion
        assert_eq!(
            diff_model.reverse_action_by_diff_hunk_index(0),
            Some((0..0, "\n".to_string()))
        );

        // Reversing the newline deletion between "def" and "ghi"
        assert_eq!(
            diff_model.reverse_action_by_diff_hunk_index(1),
            Some((2..2, "\n".to_string()))
        );

        // Reversing the deletion of "mno" followed by a newline
        assert_eq!(
            diff_model.reverse_action_by_diff_hunk_index(2),
            Some((5..5, "mno\n\n".to_string()))
        );

        // Reversing the deletion of a newline followed by "stu"
        assert_eq!(
            diff_model.reverse_action_by_diff_hunk_index(3),
            Some((8..8, "\nstu\n".to_string()))
        );

        // Reversing the trailing newline deletion
        assert_eq!(
            diff_model.reverse_action_by_diff_hunk_index(4),
            Some((11..11, "\n".to_string()))
        );
    });
}

#[test]
fn test_diff_count_before_line() {
    use warpui::App;
    App::test((), |_| async move {
        let mut diff_model = DiffModel::new();
        diff_model.set_base(
            "Hello World\nThis is the second line\n"
                .to_owned()
                .try_into()
                .unwrap(),
        );
        diff_model
            .compute_diff_for_test("Hallo World\nThis is the second line\nNew".to_string())
            .await;
        assert_eq!(diff_model.diff_hunk_count_before_line(0), 0);
        assert_eq!(diff_model.diff_hunk_count_before_line(1), 1);
        assert_eq!(diff_model.diff_hunk_count_before_line(2), 1);
    });
}

#[test]
fn test_unified_diff() {
    use warpui::App;
    App::test((), |_| async move {
        let diff = DiffModel::retrieve_unified_diff_internal(
            MultilineStr::try_new("Hello World\nThis is the second line.\nThis is the third.")
                .unwrap(),
            MultilineStr::try_new(
                "Hallo Welt\nThis is the second line.\nThis is life.\nMoar and more",
            )
            .unwrap(),
            "test.rs",
        )
        .await;
        assert_eq!(diff.unified_diff, "--- test.rs\n+++ test.rs\n@@ -1,3 +1,4 @@\n-Hello World\n+Hallo Welt\n This is the second line.\n-This is the third.\n+This is life.\n+Moar and more\n");
        assert_eq!(diff.lines_added, 3);
        assert_eq!(diff.lines_removed, 2);
    });
}

/// Test coalesce_replacements with a case where the `similar` library is known
/// to produce duplicate deletion and insertion hunks for what is logically a replacement.
#[test]
fn test_coalesce_replacements() {
    use warpui::App;
    App::test((), |_| async move {
        let mut diff_model = DiffModel::new();
        let base_text = r"
            abc
            def
            ghi

            jkl
            mno
            pqr

            stu
            vwx
            yz
        "
        .unindent();
        diff_model.set_base(MultilineString::apply(&base_text));

        // Replace with a newline:
        // * First line "abc"
        // * "ghi", which is followed by a newline
        // * "mno"
        // * "stu", which is preceded by a newline
        // * Last line "yz"
        let modified_text = r"

            def


            jkl

            pqr


            vwx

        "
        .unindent();
        diff_model.compute_diff_for_test(modified_text).await;

        assert_eq!(diff_model.diff_hunk_count(), 5);

        // Replacing "abc"
        assert_eq!(diff_model.diff_by_index(0), Some((0..1, true)));

        // Replacing "ghi"
        assert_eq!(diff_model.diff_by_index(1), Some((3..4, true)));

        // Replacing "mno"
        assert_eq!(diff_model.diff_by_index(2), Some((5..6, true)));

        // Replacing "stu"
        assert_eq!(diff_model.diff_by_index(3), Some((8..9, true)));

        // Replacing "yz"
        assert_eq!(diff_model.diff_by_index(4), Some((10..11, true)));
    });
}

/// A pure deletion (old rows removed, no new rows) on the *right* (modified) column
/// of a side-by-side diff has no rows in the right buffer, so its navigation range is
/// zero-width at `new_rows.end`. The change/replacement reverse paths cannot represent
/// it (the `RangeMap`-backed `change_mapping` panics on empty ranges, and
/// `deletion_mapping` drives gutter rendering). `compute_side_by_side_diff_data`
/// therefore records the deleted base range in `reverse_insertion_mapping`, keyed by
/// `new_rows.end`. This test mirrors exactly that `DiffStatus` and asserts the hunk can
/// now be reverted (re-inserting the deleted lines) instead of silently no-op'ing.
#[test]
fn test_reverse_action_side_by_side_pure_deletion_right_column() {
    use warpui::App;
    App::test((), |_| async move {
        let mut diff_model = DiffModel::new();
        // Base (old) content: 3 lines that will be pure-deleted in the new content.
        let base_text = "line one\nline two\nline three\nline four\n";
        diff_model.set_base(MultilineString::apply(base_text));

        // Right (modified) buffer: lines 1..3 deleted (0-indexed). The new content is
        // just "line one\nline four\n"; the deleted block occupied old rows 1..3, so on
        // the right side the zero-width navigation position is `new_rows.end == 1`.
        let old_rows = 1..3usize;
        let new_rows_end = 1usize;

        let right_status = DiffStatus::from_mappings(RangeMap::new(), HashMap::new())
            .with_deletion_hunks(vec![new_rows_end..new_rows_end])
            .with_reverse_insertion_mapping(HashMap::from([(new_rows_end, old_rows.clone())]));
        diff_model.set_status(right_status);

        // Hunk count stays aligned: exactly one hunk (the zero-width placeholder).
        assert_eq!(diff_model.diff_hunk_count(), 1);

        // Before the fix this returned `None` (silent no-op). Now it re-inserts the
        // deleted base lines at the zero-width position.
        assert_eq!(
            diff_model.reverse_action_by_diff_hunk_index(0),
            Some((1..1, "line two\nline three\n".to_string()))
        );
    });
}

/// The side-by-side left (baseline) column reports a single-line range for the clicked
/// gutter row, while the hunk's deletion range spans several rows. `diff_hunk_index_at_line`
/// must resolve any row inside the hunk back to that hunk (not count the hunk itself as
/// "before" the line, which `diff_hunk_count_before_line` would do for rows after the first).
#[test]
fn test_diff_hunk_index_at_line_multiline_hunk() {
    use warpui::App;
    App::test((), |_| async move {
        let mut diff_model = DiffModel::new();
        diff_model.set_base(MultilineString::apply(
            "line one\nline two\nline three\nline four\n",
        ));

        // Mirror the left column of a side-by-side diff: a 3-row pure-deletion hunk has
        // per-row `deletion_mapping` (1..2, 2..3, 3..4) plus one whole-hunk navigation
        // range `1..4`. The buffer (old content) is the base here.
        let mut deletion_mapping = HashMap::new();
        deletion_mapping.insert(1, 1..2);
        deletion_mapping.insert(2, 2..3);
        deletion_mapping.insert(3, 3..4);
        let left_status = DiffStatus::from_mappings(RangeMap::new(), deletion_mapping)
            .with_deletion_hunks(vec![1..4]);
        diff_model.set_status(left_status);

        // Clicking any row of the hunk (1, 2, or 3) must resolve to hunk 0.
        assert_eq!(diff_model.diff_hunk_index_at_line(1), Some(0));
        assert_eq!(diff_model.diff_hunk_index_at_line(2), Some(0));
        assert_eq!(diff_model.diff_hunk_index_at_line(3), Some(0));
        // A row outside the hunk resolves to nothing.
        assert_eq!(diff_model.diff_hunk_index_at_line(0), None);
        assert_eq!(diff_model.diff_hunk_index_at_line(4), None);
    });
}

/// Mixed-hunk side-by-side alignment (mirrors `CodeReviewView::compute_side_by_side_diff_data`):
/// every hunk contributes exactly one navigation range on each side, so a hunk index computed
/// against the left/baseline status maps to the same hunk on the right/modified status.
#[test]
fn test_side_by_side_mixed_hunk_index_alignment() {
    use warpui::App;
    App::test((), |_| async move {
        // Mixed example: hunk 0 = replacement (old row 1 -> new row 1), hunk 1 = pure deletion
        // (old row 3 -> no new row), hunk 2 = pure addition (new row 4, no old row).
        // Left/baseline status: change_mapping empty, deletion_hunks = [1..2, 3..4, 5..5].
        // Right/modified status: change_mapping = {1..2: Replacement(replaced 1..2), 4..5: Addition},
        // deletion_hunks = [1..1 (replace nav), 3..3 (deletion placeholder), 4..5? -> see below].
        //
        // To keep this readable we hand-construct statuses with the same navigation signature
        // the side-by-side computation produces: one entry per hunk per side, sorted by start.
        let mut left = DiffModel::new();
        let left_status = DiffStatus::from_mappings(RangeMap::new(), HashMap::new())
            .with_deletion_hunks(vec![1..2, 3..4, 5..5]);
        left.set_status(left_status);

        let mut right = DiffModel::new();
        let mut right_change_mapping = RangeMap::new();
        right_change_mapping.insert(
            1..2,
            ChangeType::Replacement {
                replaced_range: 1..2,
                insertion: Vec::new(),
                deletion: Vec::new(),
            },
        );
        right_change_mapping.insert(4..5, ChangeType::Addition);
        let right_status = DiffStatus::from_mappings(right_change_mapping, HashMap::new())
            .with_deletion_hunks(vec![3..3]);
        right.set_status(right_status);

        assert_eq!(left.diff_hunk_count(), 3);
        assert_eq!(right.diff_hunk_count(), 3);

        // Same index resolves on both sides.
        for index in 0..3 {
            assert!(
                left.line_range_by_diff_hunk_index(index).is_some(),
                "左列第 {index} 个 hunk 应存在"
            );
            assert!(
                right.line_range_by_diff_hunk_index(index).is_some(),
                "右列第 {index} 个 hunk 应存在"
            );
        }
    });
}
