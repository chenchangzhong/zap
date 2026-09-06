//! Text diff computation for incremental buffer updates.
//!
//! This module provides functionality for computing minimal diffs between two text strings,
//! which is used for auto-reloading files without wiping undo history or disrupting anchors.

use imara_diff::{Algorithm, Diff, InternedInput, Token};
use std::{iter, ops::Range};
use string_offset::{ByteOffset, CharOffset};

use super::buffer::{Buffer, ToBufferCharOffset};
use super::edit::TemporaryBlock;
use crate::render::model::LineCount;

/// A computed diff between two strings.
///
/// The edits are represented as byte ranges in the old text and their replacement strings.
/// These can be applied to transform the old text into the new text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextDiff {
    /// List of edits: (old_byte_range, new_text)
    pub edits: Vec<(Range<usize>, String)>,
}

impl TextDiff {
    /// Returns true if this diff represents no changes.
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// Convert byte-range edits to CharOffset-range edits.
    ///
    /// The buffer uses 1-indexed coordinates (first editable character is at CharOffset(1)).
    /// The diff byte offsets are 0-indexed relative to the plain text, so we add 1 to
    /// convert to the buffer's 1-indexed system before using to_buffer_char_offset.
    pub fn to_char_offset_edits(&self, buffer: &Buffer) -> Vec<(Range<CharOffset>, String)> {
        self.edits
            .iter()
            .map(|(byte_range, new_text)| {
                // Add 1 to convert from 0-indexed plain text byte offset to 1-indexed buffer byte offset
                let start = ByteOffset::from(byte_range.start + 1).to_buffer_char_offset(buffer);
                let end = ByteOffset::from(byte_range.end + 1).to_buffer_char_offset(buffer);
                (start..end, new_text.clone())
            })
            .collect()
    }
}

/// Compute a diff between two strings.
///
/// This uses a line-based diff algorithm (Histogram) for efficiency.
/// Returns a list of edits as (byte_range_in_old, replacement_text) pairs.
pub async fn text_diff(old_text: &str, new_text: &str) -> TextDiff {
    let input = InternedInput::new(old_text, new_text);
    // Yield here to prevent doing more work if the task is aborted.
    futures_lite::future::yield_now().await;

    // Only compute line-based diff for now. Zed does more fine-grained word-level diffing for smaller hunks
    // but I don't think it's worth it for our use case.
    let edits = diff_internal(&input, new_text).await;

    TextDiff { edits }
}

async fn diff_internal(input: &InternedInput<&str>, new_text: &str) -> Vec<(Range<usize>, String)> {
    let mut old_offset = 0;
    let mut new_offset = 0;
    let mut old_token_ix = 0;
    let mut new_token_ix = 0;
    let mut edits = Vec::new();

    let diff = Diff::compute(Algorithm::Histogram, input);

    // Yield here to prevent doing more work if the task is aborted.
    futures_lite::future::yield_now().await;

    for hunk in diff.hunks() {
        // Calculate byte offsets for unchanged tokens before this hunk
        old_offset += token_len(
            input,
            &input.before[old_token_ix as usize..hunk.before.start as usize],
        );
        new_offset += token_len(
            input,
            &input.after[new_token_ix as usize..hunk.after.start as usize],
        );

        // Calculate byte lengths of the changed tokens
        let old_len = token_len(
            input,
            &input.before[hunk.before.start as usize..hunk.before.end as usize],
        );
        let new_len = token_len(
            input,
            &input.after[hunk.after.start as usize..hunk.after.end as usize],
        );

        let old_byte_range = old_offset..old_offset + old_len;
        let new_byte_range = new_offset..new_offset + new_len;

        old_token_ix = hunk.before.end;
        new_token_ix = hunk.after.end;
        old_offset = old_byte_range.end;
        new_offset = new_byte_range.end;

        let replacement = if new_byte_range.is_empty() {
            String::new()
        } else {
            new_text[new_byte_range].to_string()
        };
        edits.push((old_byte_range, replacement));
    }

    edits
}

/// Character kind used to segment text into word-level tokens (mirrors Zed's `CharKind`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum CharKind {
    Word,
    Whitespace,
    Newline,
    Punctuation,
}

fn char_kind(c: char) -> CharKind {
    if c == '\n' {
        CharKind::Newline
    } else if c.is_whitespace() {
        CharKind::Whitespace
    } else if c.is_alphanumeric() || c == '_' {
        CharKind::Word
    } else {
        CharKind::Punctuation
    }
}

/// Segment `text` into word-level tokens (mirrors Zed's `tokenize`).
///
/// Consecutive characters of the same kind form one token; punctuation is split
/// per character (so `..` and `()` each become separate tokens).
fn tokenize(text: &str) -> impl Iterator<Item = &str> {
    let mut chars = text.char_indices();
    let mut prev: Option<(char, CharKind)> = None;
    let mut start_ix = 0;
    iter::from_fn(move || {
        for (ix, c) in chars.by_ref() {
            let mut token = None;
            let kind = char_kind(c);
            if let Some((prev_char, prev_kind)) = prev
                && (kind != prev_kind || (kind == CharKind::Punctuation && c != prev_char))
            {
                token = Some(&text[start_ix..ix]);
                start_ix = ix;
            }
            prev = Some((c, kind));
            if token.is_some() {
                return token;
            }
        }
        if start_ix < text.len() {
            let token = &text[start_ix..];
            start_ix = text.len();
            return Some(token);
        }
        None
    })
}

/// Character-level diff ranges between two strings, mirroring Zed's `word_diff_ranges`.
///
/// Returns `(old_ranges, new_ranges)`: the byte ranges (relative to each input string)
/// that were changed, with adjacent changed ranges merged.
///
/// 仍然做字符级 word diff 的最大 hunk 行数(每侧)。
///
/// 对齐 Zed 的 `MAX_WORD_DIFF_LINE_COUNT = 5`:只对「修改前后行数相等且 ≤ 该值」的
/// 纯修改 hunk 计算字符级 diff,更大的 hunk 只保留行级红/绿高亮,跳过昂贵的字级计算。
/// 取舍说明:本仓库实测 200 行 hunk ≈ release 1ms 且随行数线性叠加,几千行的大
/// modification hunk 若做字级 diff 会在打开大 diff 时卡住;完全对齐 Zed = 5 能拿到
/// 最佳打开性能,代价仅是 6 行以上的就地修改不再有字符级精细高亮(Zed 默认行为一致)。
pub const WORD_DIFF_MAX_HUNK_LINES: usize = 5;
pub fn word_diff_ranges(
    old_text: &str,
    new_text: &str,
) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let mut input: InternedInput<&str> = InternedInput::default();
    input.update_before(tokenize(old_text));
    input.update_after(tokenize(new_text));

    let mut old_ranges: Vec<Range<usize>> = Vec::new();
    let mut new_ranges: Vec<Range<usize>> = Vec::new();

    let mut old_offset = 0usize;
    let mut new_offset = 0usize;
    let mut old_token_ix = 0usize;
    let mut new_token_ix = 0usize;
    let diff = Diff::compute(Algorithm::Histogram, &input);

    for hunk in diff.hunks() {
        old_offset += token_len(
            &input,
            &input.before[old_token_ix..hunk.before.start as usize],
        );
        new_offset += token_len(
            &input,
            &input.after[new_token_ix..hunk.after.start as usize],
        );
        let old_len = token_len(
            &input,
            &input.before[hunk.before.start as usize..hunk.before.end as usize],
        );
        let new_len = token_len(
            &input,
            &input.after[hunk.after.start as usize..hunk.after.end as usize],
        );
        let old_byte_range = old_offset..old_offset + old_len;
        let new_byte_range = new_offset..new_offset + new_len;
        old_token_ix = hunk.before.end as usize;
        new_token_ix = hunk.after.end as usize;
        old_offset = old_byte_range.end;
        new_offset = new_byte_range.end;

        if !old_byte_range.is_empty() {
            if let Some(last) = old_ranges.last_mut()
                && last.end >= old_byte_range.start
            {
                last.end = old_byte_range.end;
            } else {
                old_ranges.push(old_byte_range);
            }
        }
        if !new_byte_range.is_empty() {
            if let Some(last) = new_ranges.last_mut()
                && last.end >= new_byte_range.start
            {
                last.end = new_byte_range.end;
            } else {
                new_ranges.push(new_byte_range);
            }
        }
    }

    (old_ranges, new_ranges)
}

/// Calculate total byte length of a sequence of tokens.
fn token_len(input: &InternedInput<&str>, tokens: &[Token]) -> usize {
    tokens
        .iter()
        .map(|token| input.interner[*token].len())
        .sum()
}

/// A single line-level diff hunk, mirroring Zed's `InternalDiffHunk`/`BufferDiff`.
///
/// Row ranges are 0-indexed, half-open (`start..end`) row ranges in each text.
/// Word diffs are byte ranges relative to the whole `old_text`/`new_text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineDiffHunk {
    /// Changed rows in the old text (0-indexed, exclusive end).
    pub old_rows: Range<usize>,
    /// Changed rows in the new text (0-indexed, exclusive end).
    pub new_rows: Range<usize>,
    /// Character-level deletions in the old text (byte ranges).
    pub old_word_diffs: Vec<Range<usize>>,
    /// Character-level insertions in the new text (byte ranges).
    pub new_word_diffs: Vec<Range<usize>>,
}

/// Byte offset of the start of each line (index = line number).
fn line_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        offset += line.len();
        offsets.push(offset);
    }
    offsets
}

/// Compute a line-level diff between two texts, mirroring Zed's `BufferDiff`.
///
/// Uses imara_diff Histogram with git's slider/indent post-processing so that
/// ambiguous hunks anchor at the same logical change regardless of base. For hunks
/// where old/new line counts match (modifications), also computes character-level
/// word diff ranges via [`word_diff_ranges`].
pub fn diff_lines(old_text: &str, new_text: &str) -> Vec<LineDiffHunk> {
    let input = InternedInput::new(old_text, new_text);
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_lines(&input);

    let old_offsets = line_offsets(old_text);
    let new_offsets = line_offsets(new_text);
    let mut hunks = Vec::new();

    for hunk in diff.hunks() {
        let old_rows = hunk.before.start as usize..hunk.before.end as usize;
        let new_rows = hunk.after.start as usize..hunk.after.end as usize;

        // Character-level diff only for pure modifications (equal line counts).
        //
        // Guard against pathological hunks: a thousands-of-lines "modification" (e.g. a file-wide
        // line-ending or whitespace change) would otherwise run a character-level Histogram diff
        // over the entire hunk on the main thread, freezing the UI when opening a large diff.
        // Above this size we keep the line-level red/green highlighting but skip the finer
        // character-level word diff.
        let hunk_size_is_tractable = old_rows.len() <= WORD_DIFF_MAX_HUNK_LINES;
        let (old_word_diffs, new_word_diffs) = if hunk_size_is_tractable
            && old_rows.len() == new_rows.len()
            && !new_rows.is_empty()
            && old_rows.end < old_offsets.len()
            && new_rows.end < new_offsets.len()
        {
            let old_byte_range = old_offsets[old_rows.start]..old_offsets[old_rows.end];
            let new_byte_range = new_offsets[new_rows.start]..new_offsets[new_rows.end];
            let (old_wd, new_wd) =
                word_diff_ranges(&old_text[old_byte_range.clone()], &new_text[new_byte_range.clone()]);
            let old_wd = old_wd
                .into_iter()
                .map(|r| old_byte_range.start + r.start..old_byte_range.start + r.end)
                .collect();
            let new_wd = new_wd
                .into_iter()
                .map(|r| new_byte_range.start + r.start..new_byte_range.start + r.end)
                .collect();
            (old_wd, new_wd)
        } else {
            (Vec::new(), Vec::new())
        };

        hunks.push(LineDiffHunk {
            old_rows,
            new_rows,
            old_word_diffs,
            new_word_diffs,
        });
    }

    hunks
}

/// Compute spacer blocks for each side of a side-by-side diff view.
///
/// This is a faithful port of Zed's `spacer_blocks` algorithm from
/// `crates/editor/src/display_map/block_map.rs`.
///
/// # Core Algorithm (Zed `determine_spacer`)
///
/// Zed calculates spacers by tracking a `delta` value — the difference in row
/// counts between the companion (other side) and our side. At each hunk boundary
/// we recompute this delta. When the delta *increases*, it means the companion
/// side has gained rows relative to ours, so we insert spacers on our side to
/// absorb the difference:
///
/// ```text
/// delta      = companion_rows - our_rows       (at baseline, before hunk)
/// new_delta  = companion_rows - our_rows       (at hunk end)
/// if new_delta > delta:
///     spacer_height = new_delta - delta
///     spacer goes ABOVE our_row at hunk end
/// ```
///
/// For a pure deletion (old has N rows, new has 0):
///   - After the hunk the companion has N fewer rows → delta decreases → no spacer needed on
///     the companion (right) side for the deletion itself.
///   - But *before* the hunk, we need to retroactively place spacers on the side that
///     lost rows, because the surviving content below shifted up.
///
/// In practice this simplifies to:
///   - **Left (old) side spacers** when `new_rows.len() > old_rows.len()`:
///     the right side grew, so we need `new_rows.len() - old_rows.len()` spacers
///     on the left, placed at `old_rows.end` (the row where old content resumes).
///   - **Right (new) side spacers** when `old_rows.len() > new_rows.len()`:
///     the left side grew, so we need `old_rows.len() - new_rows.len()` spacers
///     on the right, placed at `new_rows.end` (the row where new content resumes).
///   - **Pure addition** (`old_rows.is_empty()`): all new rows are additions.
///     Left gets `new_rows.len()` spacers at `new_rows.start`.
///   - **Pure deletion** (`new_rows.is_empty()`): all old rows are deletions.
///     Right gets `old_rows.len()` spacers at `old_rows.end` — the position in
///     the new text where the surviving content begins, which equals the new-side
///     row that corresponds to the first surviving row after the deletion.
///
/// 合并连续相同 insert_before 的 spacers 为一个 TemporaryBlock
///
/// 优化：将多个连续的 spacer 合并为一个，内容为多行空格（`" \n".repeat(n)`）
fn merge_consecutive_spacers(spacers: Vec<TemporaryBlock>) -> Vec<TemporaryBlock> {
    if spacers.is_empty() {
        return Vec::new();
    }

    let mut merged = Vec::new();
    let mut current_group: Vec<TemporaryBlock> = Vec::new();

    for spacer in spacers {
        if current_group.is_empty()
            || current_group.last().unwrap().insert_before == spacer.insert_before
        {
            // 当前组为空，或者 insert_before 相同，加入当前组
            current_group.push(spacer);
        } else {
            // insert_before 不同，合并当前组并开始新组
            merged.push(merge_group(&current_group));
            current_group = vec![spacer];
        }
    }

    // 合并最后一组
    if !current_group.is_empty() {
        merged.push(merge_group(&current_group));
    }

    merged
}

/// 将一组具有相同 insert_before 的 spacers 合并为一个 TemporaryBlock
fn merge_group(group: &[TemporaryBlock]) -> TemporaryBlock {
    assert!(!group.is_empty());

    let insert_before = group[0].insert_before;
    let line_decoration = group[0].line_decoration.clone();
    let inline_text_decorations = group[0].inline_text_decorations.clone();

    // 合并内容为多行空格
    let content = " \n".repeat(group.len());

    TemporaryBlock {
        content,
        insert_before,
        line_decoration,
        inline_text_decorations,
    }
}

/// The key insight from Zed: spacers are always placed at the **surviving content
/// boundary** on the side that needs them — i.e. `old_rows.end` for left-side
/// spacers, `new_rows.end` for right-side spacers (or equivalently for pure
/// addition/deletion, at the corresponding boundary).
///
/// # Arguments
/// * `diff_hunks` — line-level diff hunks from [`diff_lines`], sorted by old_rows
///
/// # Returns
/// `(left_spacers, right_spacers)` — [`TemporaryBlock`]s to insert into each editor.
pub fn compute_spacers(
    diff_hunks: &[LineDiffHunk],
) -> (Vec<TemporaryBlock>, Vec<TemporaryBlock>) {
    let mut left_spacers: Vec<TemporaryBlock> = Vec::new();
    let mut right_spacers: Vec<TemporaryBlock> = Vec::new();

    // Zed tracks a running `delta` across hunks. Between hunks, content is
    // unchanged so delta stays the same. At each hunk we compute the new delta
    // from the hunk's old/new row counts and insert spacers where it increased.
    //
    // Because our hunks are already row-indexed, the delta at any hunk boundary
    // is simply: (cumulative new rows consumed) - (cumulative old rows consumed).
    // The spacer for a hunk goes at the row where the *other* side's surviving
    // content begins — i.e. on our side, at the row after our side of the hunk.

    let mut old_consumed: usize = 0; // cumulative old rows consumed up to this hunk
    let mut new_consumed: usize = 0; // cumulative new rows consumed up to this hunk

    for hunk in diff_hunks {
        let old_count = hunk.old_rows.len();
        let new_count = hunk.new_rows.len();

        // Baseline: delta before this hunk = (new consumed) - (old consumed).
        // This reflects the row offset accumulated by previous hunks.
        let baseline_delta = new_consumed as isize - old_consumed as isize;

        // After this hunk, the delta changes based on the hunk's own row counts.
        // new_delta = (new_consumed + new_count) - (old_consumed + old_count)
        //           = baseline_delta + (new_count as isize - old_count as isize)
        let row_diff = new_count as isize - old_count as isize;
        let new_delta = baseline_delta + row_diff;

        if new_delta > baseline_delta {
            // Companion (new/right) side gained rows relative to us (old/left).
            // We need spacers on the LEFT side.
            let spacer_height = (new_delta - baseline_delta) as usize;
            // Spacer placement: above old_rows.end (where old content resumes)
            let insert_at = hunk.old_rows.end;
            for _ in 0..spacer_height {
                left_spacers.push(TemporaryBlock {
                    content: " ".to_string(),
                    insert_before: LineCount::from(insert_at),
                    line_decoration: None,
                    inline_text_decorations: Vec::new(),
                });
            }
        } else if new_delta < baseline_delta {
            // Our (old/left) side gained rows relative to companion (new/right).
            // We need spacers on the RIGHT side.
            let spacer_height = (baseline_delta - new_delta) as usize;
            // Spacer placement: above new_rows.end (where new content resumes)
            let insert_at = hunk.new_rows.end;
            for _ in 0..spacer_height {
                right_spacers.push(TemporaryBlock {
                    content: " ".to_string(),
                    insert_before: LineCount::from(insert_at),
                    line_decoration: None,
                    inline_text_decorations: Vec::new(),
                });
            }
        }
        // If new_delta == baseline_delta, no spacers needed for this hunk.

        old_consumed += old_count;
        new_consumed += new_count;
    }

    // 合并连续相同 insert_before 的 spacers
    let left_spacers = merge_consecutive_spacers(left_spacers);
    let right_spacers = merge_consecutive_spacers(right_spacers);

    (left_spacers, right_spacers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use super::super::edit::TemporaryBlock;

    #[test]
    fn word_diff_unchanged_text() {
        let (old, new) = word_diff_ranges("abc def", "abc def");
        assert!(old.is_empty());
        assert!(new.is_empty());
    }

    #[test]
    fn word_diff_single_word_change() {
        let (old, new) = word_diff_ranges("one two three", "one TWO three");
        assert_eq!(old, vec![4..7]); // "two" -> "TWO"
        assert_eq!(new, vec![4..7]);
    }

    #[test]
    fn word_diff_insertion() {
        // "ac" vs "abc" are different word tokens — the whole word is a replacement.
        let (old, new) = word_diff_ranges("ac", "abc");
        assert_eq!(old, vec![0..2]);
        assert_eq!(new, vec![0..3]);
    }

    #[test]
    fn word_diff_deletion() {
        let (old, new) = word_diff_ranges("abc", "ac");
        assert_eq!(old, vec![0..3]); // whole word deleted
        assert_eq!(new, vec![0..2]);
    }

    #[test]
    fn word_diff_replacement_with_whitespace() {
        // With a space separating words, only the changed word is highlighted.
        let (old, new) = word_diff_ranges("foo bar", "foo baz");
        assert_eq!(old, vec![4..7]); // "bar" -> "baz"
        assert_eq!(new, vec![4..7]);
    }

    #[test]
    fn word_diff_punctuation_split() {
        // Punctuation is split per character; a pure insertion inside parens is isolated.
        let (old, new) = word_diff_ranges("call_me()", "call_me(x)");
        assert!(old.is_empty());
        assert_eq!(new, vec![8..9]); // "x" inserted between "(" and ")"
    }

    #[test]
    fn word_diff_multibyte() {
        let (old, new) = word_diff_ranges("日本語", "日");
        assert_eq!(old, vec![0..9]); // whole word "日本語" (byte range)
        assert_eq!(new, vec![0..3]);
    }

    #[test]
    fn diff_lines_no_change() {
        assert!(diff_lines("a\nb\nc\n", "a\nb\nc\n").is_empty());
    }

    #[test]
    fn diff_lines_insertion() {
        // Add one line → one hunk, no word diffs (line counts differ).
        let hunks = diff_lines("a\nb\nc\n", "a\nb\nX\nc\n");
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].old_rows, 2..2); // insertion at row 2
        assert_eq!(hunks[0].new_rows, 2..3);
        assert!(hunks[0].old_word_diffs.is_empty());
        assert!(hunks[0].new_word_diffs.is_empty());
    }

    #[test]
    fn diff_lines_modification_word_diff() {
        // Modify one line in place → one hunk, word diffs present.
        let hunks = diff_lines("foo bar\n", "foo baz\n");
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].old_rows, 0..1);
        assert_eq!(hunks[0].new_rows, 0..1);
        assert_eq!(hunks[0].old_word_diffs, vec![4..7]); // "bar"
        assert_eq!(hunks[0].new_word_diffs, vec![4..7]); // "baz"
    }

    #[test]
    fn diff_lines_deletion() {
        let hunks = diff_lines("a\nb\nc\n", "a\nc\n");
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].old_rows, 1..2); // deleted row 1
        assert_eq!(hunks[0].new_rows, 1..1);
        assert!(hunks[0].old_word_diffs.is_empty());
    }

    #[test]
    fn diff_lines_merged_modification() {
        // Multi-line modification: postprocess should merge into a single MODIFY hunk.
        let hunks = diff_lines("one\ntwo\nthree\n", "one\nTWO\nTHREE\n");
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].old_rows, 1..3);
        assert_eq!(hunks[0].new_rows, 1..3);
        assert!(!hunks[0].old_word_diffs.is_empty());
        assert!(!hunks[0].new_word_diffs.is_empty());
    }

    #[test]
    fn diff_lines_huge_modification_skips_word_diff() {
        // A modification hunk above `WORD_DIFF_MAX_HUNK_LINES` must skip the character-level
        // word diff (keeping line-level highlighting) so opening very large diffs stays
        // responsive.
        let lines_before: String = (0..(WORD_DIFF_MAX_HUNK_LINES + 10))
            .map(|i| format!("line {i}\n"))
            .collect();
        let mut lines_after = lines_before.clone();
        // Modify every line so the diff merges into one big MODIFY hunk.
        lines_after = lines_after
            .lines()
            .map(|l| format!("{l} changed\n"))
            .collect();
        let hunks = diff_lines(&lines_before, &lines_after);
        assert!(!hunks.is_empty(), "should produce at least one hunk");
        for hunk in &hunks {
            if hunk.old_rows.len() > WORD_DIFF_MAX_HUNK_LINES {
                assert!(
                    hunk.old_word_diffs.is_empty() && hunk.new_word_diffs.is_empty(),
                    "超大 hunk 应跳过字符级 word diff"
                );
            }
        }
    }

    #[test]
    fn diff_lines_modified_different_counts() {
        let hunks = diff_lines("a\nb\nc\nd\ne\n", "a\nX\nY\nd\ne\n");
        assert!(!hunks.is_empty(), "should produce at least one hunk");
        let total_old: usize = hunks.iter().map(|h| h.old_rows.len()).sum();
        let total_new: usize = hunks.iter().map(|h| h.new_rows.len()).sum();
        assert_eq!(total_old, 2, "old should contribute 2 changed rows (b,c)");
        assert_eq!(total_new, 2, "new should contribute 2 changed rows (X,Y)");
    }

    #[test]
    fn diff_lines_zeds_test_basic_alignment() {
        // Zed's test_basic_alignment: 6 lines → 4 lines (bbb,ccc deleted)
        let base = "aaa\nbbb\nccc\nddd\neee\nfff\n";
        let current = "aaa\nddd\neee\nfff\n";
        let hunks = diff_lines(base, current);
        // Should produce deletion hunk(s) for bbb,ccc
        let total_old: usize = hunks.iter().map(|h| h.old_rows.len()).sum();
        let total_new: usize = hunks.iter().map(|h| h.new_rows.len()).sum();
        // bbb(1) + ccc(2) = 2 old rows deleted, 0 new rows
        assert_eq!(total_old, 2, "should delete 2 rows");
        assert_eq!(total_new, 0, "should add 0 rows");
        // Verify which rows
        for h in &hunks {
            if h.old_rows.len() > 0 && h.new_rows.len() == 0 {
                println!("deletion hunk: old_rows={:?}", h.old_rows);
            }
        }
    }

    #[test]
    fn compute_spacers_pure_addition() {
        // OLD: line 0, 1, 2
        // NEW: line 0, NEW_A, NEW_B, 1, 2 (新增两行在 line 0 之后)
        let old = "a\nb\nc\n";
        let new = "a\nX\nY\nb\nc\n";
        let hunks = diff_lines(old, new);
        
        assert_eq!(hunks.len(), 1);
        // Pure addition: old_rows is empty, new_rows is 1..3 (X and Y)
        assert!(hunks[0].old_rows.is_empty());
        assert_eq!(hunks[0].new_rows, 1..3);
        
        let (left_spacers, right_spacers) = compute_spacers(&hunks);
        
        // LHS should get 1 merged spacer (for X and Y)
        assert_eq!(left_spacers.len(), 1);
        // RHS should get 0 spacers (no deletions)
        assert_eq!(right_spacers.len(), 0);
        
        // Spacer should be inserted at position 1 (before new content starts)
        assert_eq!(left_spacers[0].insert_before, LineCount::from(1));
        // Content should be two lines of spaces
        assert_eq!(left_spacers[0].content, " \n \n");
    }

    #[test]
    fn compute_spacers_pure_deletion() {
        // OLD: line 0, 1, 2, 3
        // NEW: line 0, 3 (删除 line 1, 2)
        let old = "a\nb\nc\nd\n";
        let new = "a\nd\n";
        let hunks = diff_lines(old, new);
        
        assert_eq!(hunks.len(), 1);
        // Pure deletion: old_rows is 1..3 (b and c), new_rows is empty
        assert_eq!(hunks[0].old_rows, 1..3);
        assert!(hunks[0].new_rows.is_empty());
        
        let (left_spacers, right_spacers) = compute_spacers(&hunks);
        
        // LHS should get 0 spacers (no additions)
        assert_eq!(left_spacers.len(), 0);
        // RHS should get 1 merged spacer (for deleted b and c)
        assert_eq!(right_spacers.len(), 1);
        
        // Spacer should be inserted at position 1 (where surviving content starts)
        assert_eq!(right_spacers[0].insert_before, LineCount::from(1));
        // Content should be two lines of spaces
        assert_eq!(right_spacers[0].content, " \n \n");
    }

    #[test]
    fn compute_spacers_modification() {
        // OLD: line 0, 1, 2
        // NEW: line 0, X, Y, Z, 2 (1行变成3行)
        let old = "a\nb\nc\n";
        let new = "a\nX\nY\nZ\nc\n";
        let hunks = diff_lines(old, new);
        
        assert_eq!(hunks.len(), 1);
        // Modification: both old_rows and new_rows are non-empty
        assert_eq!(hunks[0].old_rows, 1..2);  // "b"
        assert_eq!(hunks[0].new_rows, 1..4);  // X, Y, Z
        
        let (left_spacers, right_spacers) = compute_spacers(&hunks);
        
        // LHS needs 1 merged spacer (new has 3 rows, old has 1 row)
        assert_eq!(left_spacers.len(), 1);
        // RHS needs 0 spacers
        assert_eq!(right_spacers.len(), 0);
        
        // Spacer inserted at old_rows.end = 2
        assert_eq!(left_spacers[0].insert_before, LineCount::from(2));
        // Content should be two lines of spaces
        assert_eq!(left_spacers[0].content, " \n \n");
    }

    #[test]
    fn test_multiple_temporary_blocks_with_same_insert_before() {
        // 测试多个具有相同 insert_before 的 TemporaryBlock
        // 直接调用 merge_consecutive_spacers 验证合并行为

        let blocks = vec![
            TemporaryBlock {
                content: " ".to_string(),
                insert_before: LineCount::from(5),
                line_decoration: None,
                inline_text_decorations: Vec::new(),
            },
            TemporaryBlock {
                content: " ".to_string(),
                insert_before: LineCount::from(5),
                line_decoration: None,
                inline_text_decorations: Vec::new(),
            },
            TemporaryBlock {
                content: " ".to_string(),
                insert_before: LineCount::from(5),
                line_decoration: None,
                inline_text_decorations: Vec::new(),
            },
        ];

        let merged = merge_consecutive_spacers(blocks);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].insert_before, LineCount::from(5));
        assert_eq!(merged[0].content, " \n \n \n"); // 3 行空格
    }

    #[test]
    fn merge_empty_spacers() {
        let merged = merge_consecutive_spacers(Vec::new());
        assert!(merged.is_empty());
    }

    #[test]
    fn merge_single_spacer() {
        let blocks = vec![TemporaryBlock {
            content: " ".to_string(),
            insert_before: LineCount::from(3),
            line_decoration: None,
            inline_text_decorations: Vec::new(),
        }];
        let merged = merge_consecutive_spacers(blocks);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].content, " \n");
        assert_eq!(merged[0].insert_before, LineCount::from(3));
    }

    #[test]
    fn merge_alternating_insert_before() {
        // 交替的 insert_before 不应被合并
        let blocks = vec![
            TemporaryBlock {
                content: " ".to_string(),
                insert_before: LineCount::from(1),
                line_decoration: None,
                inline_text_decorations: Vec::new(),
            },
            TemporaryBlock {
                content: " ".to_string(),
                insert_before: LineCount::from(2),
                line_decoration: None,
                inline_text_decorations: Vec::new(),
            },
            TemporaryBlock {
                content: " ".to_string(),
                insert_before: LineCount::from(1),
                line_decoration: None,
                inline_text_decorations: Vec::new(),
            },
        ];
        let merged = merge_consecutive_spacers(blocks);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].insert_before, LineCount::from(1));
        assert_eq!(merged[0].content, " \n");
        assert_eq!(merged[1].insert_before, LineCount::from(2));
        assert_eq!(merged[1].content, " \n");
        assert_eq!(merged[2].insert_before, LineCount::from(1));
        assert_eq!(merged[2].content, " \n");
    }

    #[test]
    fn test_spacer_grouping_optimization() {
        // 测试 spacer 分组优化逻辑

        // 模拟 compute_spacers 的输出：多个连续相同 insert_before 的 spacers
        let left_spacers = vec![
            TemporaryBlock {
                content: " ".to_string(),
                insert_before: LineCount::from(10),
                line_decoration: None,
                inline_text_decorations: Vec::new(),
            },
            TemporaryBlock {
                content: " ".to_string(),
                insert_before: LineCount::from(10),
                line_decoration: None,
                inline_text_decorations: Vec::new(),
            },
            TemporaryBlock {
                content: " ".to_string(),
                insert_before: LineCount::from(10),
                line_decoration: None,
                inline_text_decorations: Vec::new(),
            },
            // 另一个不同的 insert_before
            TemporaryBlock {
                content: " ".to_string(),
                insert_before: LineCount::from(20),
                line_decoration: None,
                inline_text_decorations: Vec::new(),
            },
        ];

        // 应用优化：合并连续相同 insert_before 的 spacers
        let optimized = merge_consecutive_spacers(left_spacers);

        // 验证优化结果
        assert_eq!(optimized.len(), 2);
        assert_eq!(optimized[0].insert_before, LineCount::from(10));
        assert_eq!(optimized[0].content, " \n \n \n"); // 3 行空格
        assert_eq!(optimized[1].insert_before, LineCount::from(20));
        assert_eq!(optimized[1].content, " \n"); // 1 行空格
    }

    #[test]
    fn compute_spacers_multiple_hunks_produce_grouped_output() {
        // 多个 hunk 产生的 spacers 会被正确分组合并
        // OLD: a\nb\nc\nd\ne\nf\n
        // NEW: a\nX\nY\nb\nc\nd\nZ\nW\ne\nf\n
        let old = "a\nb\nc\nd\ne\nf\n";
        let new = "a\nX\nY\nb\nc\nd\nZ\nW\ne\nf\n";
        let hunks = diff_lines(old, new);

        // 期望有 2 个 hunk：一个 for XY insertion, one for ZW insertion
        assert!(hunks.len() >= 2, "should produce at least 2 hunks");

        let (left_spacers, right_spacers) = compute_spacers(&hunks);

        // 两个 hunk 都插入到相同位置，合并为一个 spacer
        assert!(left_spacers.len() >= 1, "should have at least 1 left spacer");
        assert!(right_spacers.is_empty(), "no deletions means no right spacers");
    }

    // -----------------------------------------------------------------------
    // side-by-side 对齐不变量
    // -----------------------------------------------------------------------

    /// 模拟一列的视觉行序:`(buffer 行号, 是否 spacer)`。
    /// spacer 不占 buffer 行号,视觉上插在 `insert_before` 行之前;`insert_before == base_rows`
    /// (EOF)时追加在列尾。
    fn visual_column(base_rows: usize, spacers: &[(usize, usize)]) -> Vec<(usize, bool)> {
        let mut by_at: HashMap<usize, usize> = HashMap::new();
        for &(at, n) in spacers {
            *by_at.entry(at).or_insert(0) += n;
        }
        let mut rows = Vec::new();
        for row in 0..base_rows {
            if let Some(&n) = by_at.get(&row) {
                rows.extend(std::iter::repeat_n((row, true), n));
            }
            rows.push((row, false));
        }
        if let Some(&n) = by_at.get(&base_rows) {
            rows.extend(std::iter::repeat_n((base_rows, true), n));
        }
        rows
    }

    fn spacer_pairs(blocks: &[TemporaryBlock]) -> Vec<(usize, usize)> {
        blocks
            .iter()
            .map(|b| (b.insert_before.as_usize(), b.content.lines().count()))
            .collect()
    }

    /// 核心不变量:应用 spacers 后两列必须逐行对齐 ——
    /// 1) 两列视觉总行数相等;
    /// 2) 每个 hunk 前后的「未变行」在两列中的视觉行号一致。
    /// 这两条 together 是 side-by-side 两列对齐的充要条件(区间行数按望远镜求和闭合)。
    fn assert_columns_aligned(
        old_len: usize,
        new_len: usize,
        hunks: &[LineDiffHunk],
        context: &str,
    ) {
        let (left_blocks, right_blocks) = compute_spacers(hunks);
        let left_vis = visual_column(old_len, &spacer_pairs(&left_blocks));
        let right_vis = visual_column(new_len, &spacer_pairs(&right_blocks));

        assert_eq!(
            left_vis.len(),
            right_vis.len(),
            "{context}: 两列视觉总行数不相等 (old={old_len} new={new_len}) hunks={hunks:?} left={left_blocks:?} right={right_blocks:?}"
        );

        let find_row = |vis: &[(usize, bool)], row: usize| {
            vis.iter()
                .position(|&(r, is_spacer)| r == row && !is_spacer)
                .unwrap_or_else(|| panic!("{context}: 视觉列中找不到 buffer 行 {row}"))
        };

        let mut old_consumed = 0usize;
        let mut new_consumed = 0usize;
        for hunk in hunks {
            let gap = hunk.old_rows.start - old_consumed;
            assert_eq!(
                gap,
                hunk.new_rows.start - new_consumed,
                "{context}: hunk 前未变区长度不一致 hunks={hunks:?}"
            );
            for k in 0..gap {
                let o = old_consumed + k;
                let n = new_consumed + k;
                assert_eq!(
                    find_row(&left_vis, o),
                    find_row(&right_vis, n),
                    "{context}: 未变行错位 old 行 {o} ↔ new 行 {n} 视觉行号不同 hunks={hunks:?}"
                );
            }
            old_consumed = hunk.old_rows.end;
            new_consumed = hunk.new_rows.end;
        }
        // 末尾未变区
        assert_eq!(
            old_len - old_consumed,
            new_len - new_consumed,
            "{context}: 尾部未变区长度不一致 hunks={hunks:?}"
        );
        for k in 0..(old_len - old_consumed) {
            let o = old_consumed + k;
            let n = new_consumed + k;
            assert_eq!(
                find_row(&left_vis, o),
                find_row(&right_vis, n),
                "{context}: 尾部未变行错位 old 行 {o} ↔ new 行 {n}"
            );
        }
    }

    fn apply_random_edit(
        rng: &mut u64,
        lines: &mut Vec<String>,
        tag: &mut usize,
    ) -> Vec<String> {
        // xorshift64* —— 确定性伪随机
        *rng ^= *rng >> 12;
        *rng ^= *rng << 25;
        *rng ^= *rng >> 27;
        let r = rng.wrapping_mul(0x2545F4914F6CDD1D);
        *tag += 1;
        let kind = r % 5;
        match kind {
            0..=1 => {
                // 插入 1-3 行
                let at = (r as usize) % (lines.len() + 1);
                let count = 1 + ((r >> 8) as usize) % 3;
                let inserted: Vec<String> = (0..count)
                    .map(|i| format!("ins{}-{}", *tag, i))
                    .collect();
                lines.splice(at..at, inserted);
            }
            2 => {
                // 删除 1-3 行
                if lines.len() > 1 {
                    let at = (r as usize) % lines.len();
                    let count = 1 + ((r >> 8) as usize) % 3;
                    let end = (at + count).min(lines.len());
                    lines.drain(at..end);
                }
            }
            3 => {
                // 就地修改 1-2 行(行数不变)
                if !lines.is_empty() {
                    let at = (r as usize) % lines.len();
                    let count = 1 + ((r >> 8) as usize) % 2;
                    for i in 0..count.min(lines.len() - at) {
                        lines[at + i] = format!("mod{}-{}", *tag, i);
                    }
                }
            }
            _ => {
                // 连续「删几行再插不同行数」—— 产生行数不等的替换 hunk
                if lines.len() > 2 {
                    let at = (r as usize) % (lines.len() - 1);
                    let del = 1 + ((r >> 8) as usize) % 3;
                    let end = (at + del).min(lines.len());
                    lines.drain(at..end);
                    let ins = ((r >> 16) as usize) % 4;
                    let inserted: Vec<String> =
                        (0..ins).map(|i| format!("rep{}-{}", *tag, i)).collect();
                    lines.splice(at..at, inserted);
                }
            }
        }
        lines.clone()
    }

    #[test]
    fn side_by_side_alignment_property_random_edits() {
        let mut rng: u64 = 0x1234_5678_9ABC_DEF0;
        for case in 0..200 {
            let line_count = 1 + (case % 40);
            let mut old_lines: Vec<String> =
                (0..line_count).map(|i| format!("line {i}")).collect();
            let mut tag = 0usize;
            let edits = 1 + (case % 6);
            let mut new_lines = old_lines.clone();
            for _ in 0..edits {
                new_lines = apply_random_edit(&mut rng, &mut new_lines, &mut tag);
            }
            let old_text = old_lines.join("\n");
            let old_text = if old_text.is_empty() {
                String::new()
            } else {
                format!("{old_text}\n")
            };
            let new_text = new_lines.join("\n");
            let new_text = if new_text.is_empty() {
                String::new()
            } else {
                format!("{new_text}\n")
            };
            old_lines = old_text.lines().map(str::to_string).collect();
            new_lines = new_text.lines().map(str::to_string).collect();

            let hunks = diff_lines(&old_text, &new_text);
            assert_columns_aligned(
                old_lines.len(),
                new_lines.len(),
                &hunks,
                &format!("case {case}: old={old_text:?} new={new_text:?}"),
            );
        }
    }

    #[test]
    fn side_by_side_alignment_edge_cases() {
        // 纯增/纯删在文件头、文件尾、空文件等边界
        let cases: Vec<(&str, &str)> = vec![
            ("", "a\nb\n"),                       // 全增(空文件 → 内容)
            ("a\nb\n", ""),                       // 全删(内容 → 空文件)
            ("a\n", "X\na\n"),                    // 头部插入
            ("a\n", "a\nX\n"),                    // 尾部插入(EOF spacer)
            ("a\nb\nc\n", "a\nc\n"),              // 中间删除
            ("a\nb\nc\nd\n", "a\nX\nY\nZ\nd\n"),  // 行数不等的替换
            ("a\nb\n", "b\na\n"),                 // 换序(histogram 可能切多 hunk)
            ("a\nb\nc\nd\ne\n", "a\nb\n"),        // 尾部删除
            ("a\nb\nc\n", "\na\nb\nc\n"),         // 头部插入空行
            ("a\nb\nc\n", "a\nb\nc\n"),           // 无变化
        ];
        for (i, (old, new)) in cases.iter().enumerate() {
            let hunks = diff_lines(old, new);
            assert_columns_aligned(
                old.lines().count(),
                new.lines().count(),
                &hunks,
                &format!("edge {i}: old={old:?} new={new:?}"),
            );
        }
    }

    #[test]
    fn side_by_side_alignment_large_mixed_diff_exceeding_old_line_budget() {
        // 回归:旧实现按"spacer 总行数 > 2000"截断并把被截行折叠成 1 行占位块,
        // 两侧被截高度不同,总高差 = 新旧行数差,首个被截 hunk 之下整体错位。
        // 这里构造 spacer 总行数超过旧预算(2000)的大混合 diff,锁死对齐不变量。
        let old_lines: Vec<String> = (0..3000).map(|i| format!("line {i}")).collect();
        let mut new_lines: Vec<String> = Vec::with_capacity(old_lines.len() + 3000);
        for (i, line) in old_lines.iter().enumerate() {
            match i % 3 {
                // 每 3 行一组:2 行插入(左侧 spacer)+ 保留 1 行
                0 => {
                    new_lines.push(format!("head-a-{i}"));
                    new_lines.push(format!("head-b-{i}"));
                    new_lines.push(line.clone());
                }
                // 每 9 行一组:删除 1 行(右侧 spacer)
                1 if i % 9 == 1 => {}
                _ => new_lines.push(line.clone()),
            }
        }
        let old_text = old_lines.join("\n") + "\n";
        let new_text = new_lines.join("\n") + "\n";

        let hunks = diff_lines(&old_text, &new_text);
        assert_columns_aligned(
            old_lines.len(),
            new_lines.len(),
            &hunks,
            "大混合 diff(超旧 2000 行 spacer 预算)",
        );
    }
}
