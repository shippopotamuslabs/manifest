//! Shared structured-patch rendering: one place builds the inline
//! diff, so the transcript view and its height estimator count the
//! same lines (UI contract §2: the view reads precomputed state).
//!
//! Diff presentation (VS Code-style): added and removed lines read
//! as full-width tinted rows with dual old/new line-number gutters;
//! the background tints and gutters carry the added/removed meaning,
//! so the line text sheds its one leading prefix char. No syntax
//! highlighting (MFST-63): every line stays monochrome text in its
//! kind tone.
//! Colors come from [`crate::theme`] tokens only (ICED-UI-CONTRACT).

use iced::{Element, Length};
use manifest_domain::{PatchDocument, PatchHunk, PatchLineKind};

use crate::theme::{
    DIFF_ADDED_BG, DIFF_REMOVED_BG, FONT_SIZE_BODY, SPACE_XS, SURFACE_1, TEXT_PRIMARY,
    TEXT_SECONDARY, TEXT_TERTIARY,
};
use crate::transcript::Message;

/// Inline patch cap: at most this many diff lines render per expanded
/// tool entry; the remainder folds into one tail note.
pub const MAX_INLINE_PATCH_LINES: usize = 100;

/// Fixed width of one line-number gutter: right-aligned numbers in
/// this width keep added/removed text columns aligned across rows.
const DIFF_GUTTER_WIDTH: f32 = 48.0;

/// Hunk header text in unified-diff style, e.g. `@@ -1,8 +41,9 @@`.
fn hunk_header(old_start: u64, old_lines: u64, new_start: u64, new_lines: u64) -> String {
    format!("@@ -{old_start},{old_lines} +{new_start},{new_lines} @@")
}

/// One diff line with its resolved old/new line numbers: the pure,
/// testable row model behind [`patch_element`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffRow<'a> {
    /// 1-based old-file line number, absent on added lines and
    /// header rows.
    pub old_no: Option<u64>,
    /// 1-based new-file line number, absent on removed lines and
    /// header rows.
    pub new_no: Option<u64>,
    /// The line's classification.
    pub kind: PatchLineKind,
    /// The line's text verbatim (prefix included).
    pub text: &'a str,
}

/// Resolves per-line old/new numbers for one hunk, in document order.
/// Counters start at the hunk's `old_start`/`new_start`: context lines
/// show both numbers and advance both; removed lines show the old
/// number only and advance it; added lines show the new number only
/// and advance it. `Other` lines are unclassified content, so they
/// number like context (both shown, both advance). `Header` data
/// lines are synthetic rows: no numbers, no advance.
pub fn numbered_rows(hunk: &PatchHunk) -> Vec<DiffRow<'_>> {
    let mut old = hunk.old_start;
    let mut new = hunk.new_start;
    hunk.lines
        .iter()
        .map(|line| {
            let (old_no, new_no) = match line.kind {
                PatchLineKind::Context | PatchLineKind::Other => {
                    let numbers = (Some(old), Some(new));
                    old += 1;
                    new += 1;
                    numbers
                }
                PatchLineKind::Removed => {
                    let numbers = (Some(old), None);
                    old += 1;
                    numbers
                }
                PatchLineKind::Added => {
                    let numbers = (None, Some(new));
                    new += 1;
                    numbers
                }
                PatchLineKind::Header => (None, None),
            };
            DiffRow {
                old_no,
                new_no,
                kind: line.kind,
                text: line.text.as_str(),
            }
        })
        .collect()
}

/// Visible line text: tint and gutters carry the added/removed
/// meaning, so an added, removed, or context line sheds its one
/// leading prefix char. A line without its kind's prefix (object
/// lines carry none) and all `Other`/`Header` text stay verbatim.
fn stripped_text(kind: PatchLineKind, text: &str) -> &str {
    let prefix = match kind {
        PatchLineKind::Added => Some('+'),
        PatchLineKind::Removed => Some('-'),
        PatchLineKind::Context => Some(' '),
        PatchLineKind::Header | PatchLineKind::Other => None,
    };
    match prefix {
        Some(marker) => text.strip_prefix(marker).unwrap_or(text),
        None => text,
    }
}

/// Tone for one diff line's text: added and removed lines read
/// primary on their tints, context and other lines read secondary,
/// headers tertiary.
fn line_color(kind: PatchLineKind) -> iced::Color {
    match kind {
        PatchLineKind::Added | PatchLineKind::Removed => TEXT_PRIMARY,
        PatchLineKind::Context | PatchLineKind::Other => TEXT_SECONDARY,
        PatchLineKind::Header => TEXT_TERTIARY,
    }
}

/// Full-line background tint per kind: added and removed lines carry
/// their translucent semantic tints, everything else is transparent.
fn row_background(kind: PatchLineKind) -> Option<iced::Color> {
    match kind {
        PatchLineKind::Added => Some(DIFF_ADDED_BG),
        PatchLineKind::Removed => Some(DIFF_REMOVED_BG),
        PatchLineKind::Context | PatchLineKind::Header | PatchLineKind::Other => None,
    }
}

/// One right-aligned line-number gutter at the fixed gutter width;
/// empty while the row shows no number on that side.
fn gutter(number: Option<u64>) -> Element<'static, Message> {
    let label = number.map(|n| n.to_string()).unwrap_or_default();
    iced::widget::text(label)
        .size(FONT_SIZE_BODY)
        .color(TEXT_TERTIARY)
        .font(iced::Font::MONOSPACE)
        .width(Length::Fixed(DIFF_GUTTER_WIDTH))
        .align_x(iced::alignment::Horizontal::Right)
        .into()
}

/// One diff line as a full-width row: old gutter, new gutter, then
/// the stripped line text, wrapped in a container carrying the
/// kind's background tint. Zero padding and zero vertical spacing,
/// so one row still renders at exactly one body line height and
/// [`rendered_line_count`] keeps matching the estimator.
fn diff_line_row(row: DiffRow<'_>) -> Element<'_, Message> {
    let background = row_background(row.kind);
    let content = iced::widget::row![
        gutter(row.old_no),
        gutter(row.new_no),
        iced::widget::text(stripped_text(row.kind, row.text))
            .size(FONT_SIZE_BODY)
            .color(line_color(row.kind))
            .font(iced::Font::MONOSPACE),
    ]
    .spacing(SPACE_XS)
    .width(Length::Fill);
    iced::widget::container(content)
        .width(Length::Fill)
        .padding(0)
        .style(move |_: &iced::Theme| iced::widget::container::Style {
            background: background.map(iced::Background::Color),
            ..iced::widget::container::Style::default()
        })
        .into()
}

/// One hunk header (`@@ -a,b +c,d @@`) as a full-width tertiary row on
/// a subtle surface step. Zero padding, so it still renders at exactly
/// one body line height.
fn hunk_header_row(old_start: u64, old_lines: u64, new_start: u64, new_lines: u64) -> Element<'static, Message> {
    iced::widget::container(
        iced::widget::text(hunk_header(old_start, old_lines, new_start, new_lines))
            .size(FONT_SIZE_BODY)
            .color(TEXT_TERTIARY)
            .font(iced::Font::MONOSPACE),
    )
    .width(Length::Fill)
    .padding(0)
    .style(|_: &iced::Theme| iced::widget::container::Style {
        background: Some(iced::Background::Color(SURFACE_1)),
        ..iced::widget::container::Style::default()
    })
    .into()
}

/// Total diff body lines in document order: one path line per file,
/// one header line per hunk, then the hunk's lines.
fn body_lines(doc: &PatchDocument) -> usize {
    doc.files
        .iter()
        .map(|file| {
            1 + file
                .hunks
                .iter()
                .map(|hunk| 1 + hunk.lines.len())
                .sum::<usize>()
        })
        .sum()
}

/// Rendered text-line count for [`patch_element`] under `limit`: the
/// shown body lines plus one tail note line while truncated. The
/// height estimator multiplies this by the body line height, so any
/// change here must move the estimator with it.
pub fn rendered_line_count(doc: &PatchDocument, limit: usize) -> usize {
    let total = body_lines(doc);
    let shown = total.min(limit);
    if total > shown { shown + 1 } else { shown }
}

/// Renders one expanded patch document: path lines, hunk headers,
/// and capped diff lines, then a `… N more lines` tail while
/// truncated. Pure view: no state, no fetch.
pub fn patch_element(doc: &PatchDocument, limit: usize) -> Element<'_, Message> {
    let total = body_lines(doc);
    let mut shown = 0;
    let mut column = iced::widget::column![].width(Length::Fill);
    'files: for file in &doc.files {
        if shown >= limit {
            break;
        }
        column = column.push(
            iced::widget::text(file.path.as_str())
                .size(FONT_SIZE_BODY)
                .color(TEXT_PRIMARY)
                .font(iced::Font::MONOSPACE),
        );
        shown += 1;
        for hunk in &file.hunks {
            if shown >= limit {
                break 'files;
            }
            column = column.push(hunk_header_row(
                hunk.old_start,
                hunk.old_lines,
                hunk.new_start,
                hunk.new_lines,
            ));
            shown += 1;
            for row in numbered_rows(hunk) {
                if shown >= limit {
                    break 'files;
                }
                column = column.push(diff_line_row(row));
                shown += 1;
            }
        }
    }
    if total > shown {
        column = column.push(
            iced::widget::text(format!("… {} more lines", total - shown))
                .size(FONT_SIZE_BODY)
                .color(TEXT_TERTIARY),
        );
    }
    column.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use manifest_domain::{PatchFile, PatchLine};

    fn line(kind: PatchLineKind, text: &str) -> PatchLine {
        PatchLine {
            kind,
            text: String::from(text),
        }
    }

    fn hunk(old_start: u64, new_start: u64, lines: Vec<PatchLine>) -> PatchHunk {
        PatchHunk {
            old_start,
            old_lines: 0,
            new_start,
            new_lines: 0,
            lines,
        }
    }

    fn doc() -> PatchDocument {
        PatchDocument {
            files: vec![PatchFile {
                path: String::from("src/main.rs"),
                hunks: vec![PatchHunk {
                    old_start: 1,
                    old_lines: 3,
                    new_start: 1,
                    new_lines: 3,
                    lines: vec![
                        line(PatchLineKind::Context, " context"),
                        line(PatchLineKind::Removed, "-old"),
                        line(PatchLineKind::Added, "+new"),
                    ],
                }],
            }],
        }
    }

    #[test]
    fn counts_path_header_and_lines_without_cap() {
        // 1 path + 1 hunk header + 3 lines, no tail.
        assert_eq!(rendered_line_count(&doc(), MAX_INLINE_PATCH_LINES), 5);
    }

    #[test]
    fn cap_folds_the_rest_into_one_tail_line() {
        // Limit 2 shows 2 body lines plus the tail note.
        assert_eq!(rendered_line_count(&doc(), 2), 3);
        // The capped element still builds.
        let _ = patch_element(&doc(), 2);
    }

    #[test]
    fn empty_document_renders_no_lines() {
        let empty = PatchDocument { files: Vec::new() };
        assert_eq!(rendered_line_count(&empty, MAX_INLINE_PATCH_LINES), 0);
        let _ = patch_element(&empty, MAX_INLINE_PATCH_LINES);
    }

    #[test]
    fn numbering_tracks_old_and_new_counters() {
        let hunk = hunk(
            10,
            20,
            vec![
                line(PatchLineKind::Context, " ctx"),
                line(PatchLineKind::Removed, "-old"),
                line(PatchLineKind::Removed, "-old2"),
                line(PatchLineKind::Added, "+new"),
                line(PatchLineKind::Context, " end"),
            ],
        );
        let rows = numbered_rows(&hunk);
        let numbers: Vec<(Option<u64>, Option<u64>)> =
            rows.iter().map(|row| (row.old_no, row.new_no)).collect();
        assert_eq!(
            numbers,
            vec![
                (Some(10), Some(20)),
                (Some(11), None),
                (Some(12), None),
                (None, Some(21)),
                (Some(13), Some(22)),
            ]
        );
    }

    #[test]
    fn numbering_resets_at_each_hunk_start() {
        let first_hunk = hunk(1, 1, vec![line(PatchLineKind::Context, " a")]);
        let second_hunk = hunk(40, 44, vec![line(PatchLineKind::Context, " b")]);
        let first = numbered_rows(&first_hunk);
        let second = numbered_rows(&second_hunk);
        assert_eq!((first[0].old_no, first[0].new_no), (Some(1), Some(1)));
        assert_eq!((second[0].old_no, second[0].new_no), (Some(40), Some(44)));
    }

    #[test]
    fn header_rows_show_no_numbers_and_other_advances_both() {
        let hunk = hunk(
            5,
            7,
            vec![
                line(PatchLineKind::Header, "@@ -5 +7 @@"),
                line(PatchLineKind::Other, "meta"),
                line(PatchLineKind::Context, " ctx"),
            ],
        );
        let rows = numbered_rows(&hunk);
        assert_eq!((rows[0].old_no, rows[0].new_no), (None, None));
        // Other numbers like context: both sides advance.
        assert_eq!((rows[1].old_no, rows[1].new_no), (Some(5), Some(7)));
        assert_eq!((rows[2].old_no, rows[2].new_no), (Some(6), Some(8)));
    }

    #[test]
    fn prefix_stripping_sheds_one_marker_and_keeps_the_rest_verbatim() {
        assert_eq!(stripped_text(PatchLineKind::Added, "+new"), "new");
        assert_eq!(stripped_text(PatchLineKind::Removed, "-old"), "old");
        assert_eq!(stripped_text(PatchLineKind::Context, " ctx"), "ctx");
        // Lines without their kind's marker (typeless object lines)
        // and Other/Header text stay verbatim.
        assert_eq!(stripped_text(PatchLineKind::Added, "bare"), "bare");
        assert_eq!(stripped_text(PatchLineKind::Context, ""), "");
        assert_eq!(stripped_text(PatchLineKind::Other, "-x"), "-x");
        assert_eq!(
            stripped_text(PatchLineKind::Header, "@@ -1 +1 @@"),
            "@@ -1 +1 @@"
        );
    }

    #[test]
    fn tint_mapping_covers_every_kind() {
        assert_eq!(row_background(PatchLineKind::Added), Some(DIFF_ADDED_BG));
        assert_eq!(row_background(PatchLineKind::Removed), Some(DIFF_REMOVED_BG));
        assert_eq!(row_background(PatchLineKind::Context), None);
        assert_eq!(row_background(PatchLineKind::Header), None);
        assert_eq!(row_background(PatchLineKind::Other), None);
    }

    #[test]
    fn tinted_lines_read_primary_without_sign_markers() {
        // Tints and gutters carry added/removed meaning; no sign
        // cell or per-kind marker helpers remain.
        assert_eq!(line_color(PatchLineKind::Added), TEXT_PRIMARY);
        assert_eq!(line_color(PatchLineKind::Removed), TEXT_PRIMARY);
        assert_eq!(line_color(PatchLineKind::Context), TEXT_SECONDARY);
        assert_eq!(line_color(PatchLineKind::Other), TEXT_SECONDARY);
        assert_eq!(line_color(PatchLineKind::Header), TEXT_TERTIARY);
        // Stripped text must not regain `+`/`-` prefixes.
        assert_eq!(stripped_text(PatchLineKind::Added, "+new"), "new");
        assert_eq!(stripped_text(PatchLineKind::Removed, "-old"), "old");
    }

    #[test]
    fn multi_hunk_count_still_matches_visual_rows() {
        let multi = PatchDocument {
            files: vec![PatchFile {
                path: String::from("src/main.rs"),
                hunks: vec![
                    hunk(1, 1, vec![line(PatchLineKind::Context, " a")]),
                    hunk(40, 44, vec![line(PatchLineKind::Added, "+b")]),
                ],
            }],
        };
        // 1 path + (1 header + 1 line) * 2 hunks, no tail.
        assert_eq!(rendered_line_count(&multi, MAX_INLINE_PATCH_LINES), 5);
        let _ = patch_element(&multi, MAX_INLINE_PATCH_LINES);
        // Capped mid-second-hunk: shown rows plus one tail note.
        assert_eq!(rendered_line_count(&multi, 3), 4);
        let _ = patch_element(&multi, 3);
    }
}
