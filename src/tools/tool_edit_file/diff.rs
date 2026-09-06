use crate::tools::utils::text::utf8_prefix;

const MAX_DIFF_BYTES: usize = 32 * 1024;
const DIFF_CONTEXT_LINES: usize = 3;
const DIFF_TRUNCATION_MESSAGE: &str = "[diff truncated: output is bounded by server limits]";

/// Bounded unified diff returned after a successful edit.
pub(super) struct Diff {
    /// Unified diff content, never larger than the server response budget.
    pub(super) content: String,
    /// Whether the diff ended with the explicit truncation marker.
    pub(super) truncated: bool,
}

/// Builds a UTF-8-safe unified diff around the exact replacement.
pub(super) fn bounded_diff(
    path: &str,
    original: &str,
    updated: &str,
    start: usize,
    old_text: &str,
    new_text: &str,
) -> Diff {
    let full = unified_diff(path, original, updated, start, old_text, new_text);
    if full.len() <= MAX_DIFF_BYTES {
        return Diff {
            content: full,
            truncated: false,
        };
    }
    let limit = MAX_DIFF_BYTES.saturating_sub(DIFF_TRUNCATION_MESSAGE.len() + 1);
    let end = utf8_prefix(&full, limit);
    Diff {
        content: format!("{}\n{}", &full[..end], DIFF_TRUNCATION_MESSAGE),
        truncated: true,
    }
}

fn unified_diff(
    path: &str,
    original: &str,
    updated: &str,
    start: usize,
    old_text: &str,
    new_text: &str,
) -> String {
    let old_lines = original.split('\n').collect::<Vec<_>>();
    let new_lines = updated.split('\n').collect::<Vec<_>>();
    let old_start = line_number(original, start);
    let new_start = line_number(updated, start);
    let old_count = changed_line_count(old_text);
    let new_count = changed_line_count(new_text);
    let before = old_start.saturating_sub(1).min(DIFF_CONTEXT_LINES);
    let old_changed_start = old_start.saturating_sub(1);
    let new_changed_start = new_start.saturating_sub(1);
    let old_changed_end = (old_changed_start + old_count).min(old_lines.len());
    let new_changed_end = (new_changed_start + new_count).min(new_lines.len());
    let after_end = (new_changed_end + DIFF_CONTEXT_LINES).min(new_lines.len());
    let old_hunk_count = before
        + old_changed_end.saturating_sub(old_changed_start)
        + after_end.saturating_sub(new_changed_end);
    let new_hunk_count = before
        + new_changed_end.saturating_sub(new_changed_start)
        + after_end.saturating_sub(new_changed_end);
    let mut diff = format!(
        "--- {path}\n+++ {path}\n@@ -{},{} +{},{} @@\n",
        old_start - before,
        old_hunk_count,
        new_start - before,
        new_hunk_count
    );
    append_lines(
        &mut diff,
        &old_lines[old_changed_start - before..old_changed_start],
        ' ',
    );
    append_lines(
        &mut diff,
        &old_lines[old_changed_start..old_changed_end],
        '-',
    );
    append_lines(
        &mut diff,
        &new_lines[new_changed_start..new_changed_end],
        '+',
    );
    append_lines(&mut diff, &new_lines[new_changed_end..after_end], ' ');
    diff
}

fn append_lines(output: &mut String, lines: &[&str], prefix: char) {
    for line in lines {
        output.push(prefix);
        output.push_str(line);
        output.push('\n');
    }
}

fn line_number(text: &str, byte_offset: usize) -> usize {
    text[..byte_offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

/// Counts physical lines touched by a replacement, including an empty deletion.
pub(super) fn changed_line_count(text: &str) -> usize {
    text.bytes().filter(|byte| *byte == b'\n').count() + usize::from(!text.ends_with('\n'))
}

#[cfg(test)]
/// Test-visible copy of the production diff response cap.
pub(super) const TEST_MAX_DIFF_BYTES: usize = MAX_DIFF_BYTES;
