//! Shared structured-patch rendering: one place builds the inline
//! diff, so the transcript view and its height estimator count the
//! same lines (UI contract §2: the view reads precomputed state).
//!
//! Diff presentation (VS Code-style): added and removed lines read
//! as full-width tinted rows with dual old/new line-number gutters;
//! the background tints and gutters carry the added/removed meaning,
//! so the line text sheds its one leading prefix char. Token
//! foreground colors (MFST-63) render over the tints: one shared
//! renderer maps cached [`HighlightedPatch`] spans to theme tokens,
//! falling back to the kind tone where a span carries no token.
//! Colors come from [`crate::theme`] tokens only (ICED-UI-CONTRACT).

use std::sync::OnceLock;

use iced::{Element, Length};
use manifest_domain::{PatchDocument, PatchHunk, PatchLineKind};
use syntect::parsing::{ParseState, ScopeStack, SyntaxDefinition, SyntaxReference, SyntaxSet};

use crate::theme::{
    DIFF_ADDED_BG, DIFF_REMOVED_BG, FONT_SIZE_BODY, SPACE_XS, SURFACE_1, SYNTAX_COMMENT,
    SYNTAX_CONSTANT, SYNTAX_FUNCTION, SYNTAX_KEYWORD, SYNTAX_NUMBER, SYNTAX_STRING, SYNTAX_TYPE,
    TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY,
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

/// One syntax token category for a highlighted span (MFST-63): the
/// shared renderer maps these to [`crate::theme`] `SYNTAX_*` tokens,
/// so token colors live in the theme exactly once. `Plain` carries
/// no token color: the caller falls back to [`line_color`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxCategory {
    Keyword,
    String,
    Comment,
    Type,
    Function,
    Number,
    Constant,
    Plain,
}

/// One highlighted diff line: foreground spans over the stripped
/// line text (the diff prefix is already shed, see
/// [`stripped_text`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightedLine {
    pub spans: Vec<(String, SyntaxCategory)>,
}

/// One hunk's highlighted lines, in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightedHunk {
    pub lines: Vec<HighlightedLine>,
}

/// One file's highlight result: the resolved language plus one
/// highlighted hunk per document hunk, in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightedFile {
    /// The resolved syntax name, or `None` when the path resolves
    /// to no known syntax (the file stays plain). Provenance for
    /// tests and diagnostics; the renderer never branches on it.
    #[allow(dead_code)]
    pub language: Option<String>,
    pub hunks: Vec<HighlightedHunk>,
}

/// Highlighted form of a [`PatchDocument`]: one entry per document
/// file, each mirroring that file's hunk/line structure. Computed
/// once per fetched patch and cached with it (the transcript cache
/// owns the single call); the renderer only reads this value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightedPatch {
    pub files: Vec<HighlightedFile>,
}

/// Combined syntax set: syntect defaults plus the vendored extras in
/// `assets/syntax/`, built once. Defaults load first so the extras
/// only fill gaps, never shadow a built-in (mirrors the step-1 probe
/// test's construction).
fn syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(|| {
        let mut builder = SyntaxSet::load_defaults_nonewlines().into_builder();
        for source in [
            include_str!("../assets/syntax/TypeScript.sublime-syntax"),
            include_str!("../assets/syntax/Kotlin.sublime-syntax"),
            include_str!("../assets/syntax/TOML.sublime-syntax"),
        ] {
            // A vendored file that fails to parse is skipped: the
            // affected language falls back to plain, never a panic.
            if let Ok(definition) = SyntaxDefinition::load_from_str(source, true, None) {
                builder.add(definition);
            }
        }
        builder.build()
    })
}

/// File extension for syntax lookup: the suffix after the last `.`
/// of the path's final segment, lowercased. `.mjs`/`.cjs` alias to
/// the built-in JavaScript syntax's `js` lookup (no vendored file:
/// see the step-1 probe test's doc comment for why).
fn syntax_extension(path: &str) -> Option<String> {
    let file = path.rsplit('/').next().unwrap_or(path);
    let (_, extension) = file.rsplit_once('.')?;
    if extension.is_empty() {
        return None;
    }
    let lower = extension.to_ascii_lowercase();
    match lower.as_str() {
        "mjs" | "cjs" => Some(String::from("js")),
        _ => Some(lower),
    }
}

/// Syntax for one patch path, or `None` for an unknown language.
fn syntax_for_path<'s>(set: &'s SyntaxSet, path: &str) -> Option<&'s SyntaxReference> {
    set.find_syntax_by_extension(syntax_extension(path)?.as_str())
}

/// Syntax for one markdown fence token, or `None` for an empty or
/// unknown language. Resolution order over the trimmed token: syntax
/// name (case-insensitive scan, so `Python` and `python` match), then
/// extension (so `py`, `rs`, `ts`, `js`, `kt`, `md` match). Fence
/// tokens name languages, not files, so the `.mjs`/`.cjs` path alias
/// does not apply here.
fn syntax_for_language<'s>(
    set: &'s SyntaxSet,
    language: Option<&str>,
) -> Option<&'s SyntaxReference> {
    let token = language?.trim();
    if token.is_empty() {
        return None;
    }
    let lower = token.to_ascii_lowercase();
    if let Some(syntax) = set
        .syntaxes()
        .iter()
        .find(|syntax| syntax.name.to_ascii_lowercase() == lower)
    {
        return Some(syntax);
    }
    set.find_syntax_by_extension(lower.as_str())
}

/// Highlights one fenced code block: one syntect pass over the code
/// lines in order, so multi-line constructs (block comments,
/// multi-line strings) tokenize across line boundaries. Returns one
/// span row per code line; lines join back to the input verbatim.
/// An empty, missing, or unknown language highlights every line as
/// one plain span. Never panics: a line that fails to parse degrades
/// to plain.
pub fn highlight_code(
    language: Option<&str>,
    code: &str,
) -> Vec<Vec<(String, SyntaxCategory)>> {
    let plain_row = |line: &str| vec![(String::from(line), SyntaxCategory::Plain)];
    let set = syntax_set();
    let Some(syntax) = syntax_for_language(set, language) else {
        return code
            .split('\n')
            .map(|line| plain_row(line.strip_suffix('\r').unwrap_or(line)))
            .collect();
    };
    let mut state = ParseState::new(syntax);
    code.split('\n')
        .map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            tokenize_line(set, &mut state, line)
        })
        .collect()
}

/// Classifies one scope-stack snapshot by scope prefix, innermost
/// scope first: `comment` scopes read as comments, `string` as
/// strings, keyword/control as keywords, storage and type-name
/// scopes as types, function-name scopes as functions, numeric
/// constants as numbers, other language constants as constants,
/// Markdown headings as types, Markdown raw/code as strings,
/// Markdown link targets as constants, and everything else as
/// plain. Markdown emphasis without a color expression (bold,
/// italic, quotes, list markers) stays deliberately unmapped, so
/// it resolves outward (keeping e.g. a heading's color) or lands
/// plain: no false coloring.
fn classify_stack(stack: &ScopeStack) -> SyntaxCategory {
    for scope in stack.as_slice().iter().rev() {
        let name = scope.to_string();
        let stem = name.as_str();
        if let Some(category) = classify_scope_name(stem) {
            return category;
        }
    }
    SyntaxCategory::Plain
}

/// Classifies one dotted scope name by prefix. Returns `None` while
/// the name matches no token category, so the caller keeps walking
/// outward through the stack. Function-declaration keywords (`fn`,
/// `def`, `function`) scope as `storage.type.function` but read as
/// keywords, not type names, so they match before the storage rule.
fn classify_scope_name(name: &str) -> Option<SyntaxCategory> {
    if name == "comment" || name.starts_with("comment.") {
        Some(SyntaxCategory::Comment)
    } else if name == "string" || name.starts_with("string.") {
        Some(SyntaxCategory::String)
    } else if name == "keyword"
        || name.starts_with("keyword.")
        || name == "control"
        || name.starts_with("control.")
        || name == "storage.type.function"
        || name.starts_with("storage.type.function.")
    {
        Some(SyntaxCategory::Keyword)
    } else if name == "storage"
        || name.starts_with("storage.")
        || name.starts_with("entity.name.type")
        || name.starts_with("support.class")
        || name.starts_with("support.type")
    {
        Some(SyntaxCategory::Type)
    } else if name.starts_with("entity.name.function")
        || name.starts_with("entity.name.method")
        || name.starts_with("support.function")
    {
        Some(SyntaxCategory::Function)
    } else if name.starts_with("constant.numeric") {
        Some(SyntaxCategory::Number)
    } else if name == "constant"
        || name.starts_with("constant.language")
        || name.starts_with("constant.character")
        || name.starts_with("constant.other")
    {
        Some(SyntaxCategory::Constant)
    } else if name == "markup.heading" || name.starts_with("markup.heading.") {
        // Headings (`markup.heading.1` … `.6`) are document
        // structure: the Type token gives them prominence without
        // confusing them with code keywords. The heading text itself
        // scopes as `entity.name.section`, which stays unmapped so
        // the walk reaches this rule.
        Some(SyntaxCategory::Type)
    } else if name == "markup.raw" || name.starts_with("markup.raw.") {
        // Inline code spans (`markup.raw.inline`) and fenced code
        // blocks (`markup.raw.block`) read as code text.
        Some(SyntaxCategory::String)
    } else if name == "markup.underline.link" || name.starts_with("markup.underline.link.") {
        // Inline/autolink targets (`markup.underline.link`, image
        // targets `markup.underline.link.image`) are constant
        // reference values. Reference ids already land on Constant
        // through `constant.other`. Bold, italic, quote, and list
        // scopes (`markup.bold`, `markup.italic`, `markup.quote`,
        // `markup.list…`) stay deliberately unmapped: color cannot
        // express emphasis, and leaving them unmapped lets emphasis
        // inside a heading keep the heading's color.
        Some(SyntaxCategory::Constant)
    } else {
        None
    }
}

/// Appends one span, merging it into the previous span while the
/// category matches. Empty text contributes no span.
fn push_span(spans: &mut Vec<(String, SyntaxCategory)>, text: &str, category: SyntaxCategory) {
    if text.is_empty() {
        return;
    }
    if let Some((last_text, last_category)) = spans.last_mut()
        && *last_category == category
    {
        last_text.push_str(text);
        return;
    }
    spans.push((String::from(text), category));
}

/// One plain line: a single span over the stripped text.
fn plain_line(stripped: &str) -> HighlightedLine {
    HighlightedLine {
        spans: vec![(String::from(stripped), SyntaxCategory::Plain)],
    }
}

/// Tokenizes one line through the running [`ParseState`]: the shared
/// syntect core behind [`highlight_line`] and [`highlight_code`].
/// Feeds the text plus the trailing newline that
/// `load_defaults_nonewlines` requires; the newline never reaches
/// the spans. A line that fails to parse degrades to one plain span.
/// Never panics on syntect output: unparsable or unsliceable lines
/// degrade to plain.
fn tokenize_line(set: &SyntaxSet, state: &mut ParseState, text: &str) -> Vec<(String, SyntaxCategory)> {
    let plain = || vec![(String::from(text), SyntaxCategory::Plain)];
    let probed = format!("{text}\n");
    let operations = match state.parse_line(&probed, set) {
        Ok(operations) => operations,
        Err(_) => return plain(),
    };
    let mut stack = ScopeStack::new();
    let mut spans: Vec<(String, SyntaxCategory)> = Vec::new();
    let mut start = 0;
    for (index, operation) in &operations {
        let index = (*index).min(probed.len());
        if index > start {
            match probed.get(start..index) {
                Some(segment) => push_span(&mut spans, segment, classify_stack(&stack)),
                None => return plain(),
            }
        }
        let _ = stack.apply(operation);
        start = index;
    }
    if start < probed.len() {
        match probed.get(start..) {
            Some(segment) => push_span(&mut spans, segment, classify_stack(&stack)),
            None => return plain(),
        }
    }
    // Shed the probed trailing newline from the final span; an
    // all-newline line (empty text) stays one empty span.
    if let Some((last_text, _)) = spans.last_mut()
        && last_text.ends_with('\n')
    {
        last_text.pop();
    }
    if spans.is_empty() {
        return plain();
    }
    spans
}

/// Highlights one diff line through the file's running [`ParseState`].
/// The stripped text (prefix shed, see [`stripped_text`]) runs through
/// the shared [`tokenize_line`] core. `Header` metadata lines (e.g.
/// `diff --git`, `index`) never enter the syntax pass: they stay one
/// plain span so the renderer falls back to the tertiary header tone.
fn highlight_line(
    set: &SyntaxSet,
    state: &mut ParseState,
    kind: PatchLineKind,
    text: &str,
) -> HighlightedLine {
    let stripped = stripped_text(kind, text);
    if kind == PatchLineKind::Header {
        return plain_line(stripped);
    }
    HighlightedLine {
        spans: tokenize_line(set, state, stripped),
    }
}

/// Highlights one file: one syntect pass over all its hunks in
/// document order, so multi-line constructs (block comments,
/// multi-line strings) tokenize across hunk boundaries.
fn highlight_file(set: &SyntaxSet, file: &manifest_domain::PatchFile) -> HighlightedFile {
    let Some(syntax) = syntax_for_path(set, &file.path) else {
        return HighlightedFile {
            language: None,
            hunks: file
                .hunks
                .iter()
                .map(|hunk| HighlightedHunk {
                    lines: hunk
                        .lines
                        .iter()
                        .map(|line| plain_line(stripped_text(line.kind, &line.text)))
                        .collect(),
                })
                .collect(),
        };
    };
    let language = Some(syntax.name.clone());
    let mut state = ParseState::new(syntax);
    let hunks = file
        .hunks
        .iter()
        .map(|hunk| HighlightedHunk {
            lines: hunk
                .lines
                .iter()
                .map(|line| highlight_line(set, &mut state, line.kind, &line.text))
                .collect(),
        })
        .collect();
    HighlightedFile { language, hunks }
}

/// Highlights one patch document: one syntect pass per file, in
/// document order. Files whose path resolves to no syntax highlight
/// every line as a single plain span of the stripped text. Never
/// panics on syntect output: unparsable lines degrade to plain.
pub fn highlight_document(doc: &PatchDocument) -> HighlightedPatch {
    let set = syntax_set();
    HighlightedPatch {
        files: doc.files.iter().map(|file| highlight_file(set, file)).collect(),
    }
}

/// Token color for one syntax category: `Plain` carries none, so the
/// caller falls back to [`line_color`] for the diff kind. Diff
/// meaning stays in the tints and gutters; this only tints the
/// token foreground.
pub fn category_color(category: SyntaxCategory) -> Option<iced::Color> {
    match category {
        SyntaxCategory::Keyword => Some(SYNTAX_KEYWORD),
        SyntaxCategory::String => Some(SYNTAX_STRING),
        SyntaxCategory::Comment => Some(SYNTAX_COMMENT),
        SyntaxCategory::Type => Some(SYNTAX_TYPE),
        SyntaxCategory::Function => Some(SYNTAX_FUNCTION),
        SyntaxCategory::Number => Some(SYNTAX_NUMBER),
        SyntaxCategory::Constant => Some(SYNTAX_CONSTANT),
        SyntaxCategory::Plain => None,
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
/// the stripped line text as token spans over the kind's tone, all
/// wrapped in a container carrying the kind's background tint. Zero
/// padding and zero vertical spacing, so one row still renders at
/// exactly one body line height and [`rendered_line_count`] keeps
/// matching the estimator.
fn diff_line_row<'a>(
    row: DiffRow<'a>,
    highlighted: Option<&'a HighlightedLine>,
) -> Element<'a, Message> {
    let background = row_background(row.kind);
    let fallback = line_color(row.kind);
    // Owned span values over borrowed text: cached spans borrow the
    // highlight cache, while a missing (or empty) cache line falls
    // back to one plain span borrowing the stripped document line.
    // Both sources outlive the element.
    let spans: Vec<iced::widget::text::Span<'a, (), iced::Font>> = match highlighted {
        Some(line) if !line.spans.is_empty() => line
            .spans
            .iter()
            .map(|(text, category)| {
                iced::widget::span(text.as_str())
                    .color(category_color(*category).unwrap_or(fallback))
                    .font(iced::Font::MONOSPACE)
            })
            .collect(),
        _ => vec![
            iced::widget::span(stripped_text(row.kind, row.text))
                .color(fallback)
                .font(iced::Font::MONOSPACE),
        ],
    };
    let content = iced::widget::row![
        gutter(row.old_no),
        gutter(row.new_no),
        iced::widget::rich_text(spans).size(FONT_SIZE_BODY),
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

/// Cached highlight lookup: the line at the same file/hunk/line
/// index, or `None` while the cache shape drifts from the document.
/// A `None` degrades to one plain span, so the view never panics on
/// a stale cache.
fn cached_line(
    highlighted: &HighlightedPatch,
    file: usize,
    hunk: usize,
    line: usize,
) -> Option<&HighlightedLine> {
    highlighted
        .files
        .get(file)?
        .hunks
        .get(hunk)?
        .lines
        .get(line)
}

/// Renders one expanded patch document: path lines, hunk headers,
/// and capped diff lines, then a `… N more lines` tail while
/// truncated. Pure view: no state, no fetch, and no highlighting
/// work — `highlighted` arrives precomputed (see
/// [`highlight_document`]; the transcript cache owns the one call
/// per fetched patch). A cache line that fails to resolve renders
/// as one plain span in the kind tone.
pub fn patch_element<'a>(
    doc: &'a PatchDocument,
    highlighted: &'a HighlightedPatch,
    limit: usize,
) -> Element<'a, Message> {
    let total = body_lines(doc);
    let mut shown = 0;
    let mut column = iced::widget::column![].width(Length::Fill);
    'files: for (file_index, file) in doc.files.iter().enumerate() {
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
        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
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
            for (line_index, row) in numbered_rows(hunk).iter().enumerate() {
                if shown >= limit {
                    break 'files;
                }
                let cached = cached_line(highlighted, file_index, hunk_index, line_index);
                column = column.push(diff_line_row(*row, cached));
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
        // The capped element still builds over the cached model.
        let doc = doc();
        let highlighted = highlight_document(&doc);
        let _ = patch_element(&doc, &highlighted, 2);
    }

    #[test]
    fn empty_document_renders_no_lines() {
        let empty = PatchDocument { files: Vec::new() };
        let highlighted = highlight_document(&empty);
        assert_eq!(rendered_line_count(&empty, MAX_INLINE_PATCH_LINES), 0);
        let _ = patch_element(&empty, &highlighted, MAX_INLINE_PATCH_LINES);
    }

    #[test]
    fn element_builds_over_highlighted_rust_spans() {
        // The cached model carries token spans (not plain), and the
        // element builds over it without highlighting work of its own.
        let doc = PatchDocument {
            files: vec![PatchFile {
                path: String::from("src/main.rs"),
                hunks: vec![hunk(1, 1, vec![line(PatchLineKind::Added, "+fn main() {")])],
            }],
        };
        let highlighted = highlight_document(&doc);
        assert!(highlighted.files.iter().any(|file| file.hunks.iter().any(
            |hunk| hunk.lines.iter().any(|line| line
                .spans
                .iter()
                .any(|(_, category)| *category != SyntaxCategory::Plain))
        )));
        let _ = patch_element(&doc, &highlighted, MAX_INLINE_PATCH_LINES);
        // A drifted cache (fewer files than the document) still
        // builds: missing lines fall back to one plain span.
        let stale = HighlightedPatch { files: Vec::new() };
        let _ = patch_element(&doc, &stale, MAX_INLINE_PATCH_LINES);
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
        let highlighted = highlight_document(&multi);
        assert_eq!(rendered_line_count(&multi, MAX_INLINE_PATCH_LINES), 5);
        let _ = patch_element(&multi, &highlighted, MAX_INLINE_PATCH_LINES);
        // Capped mid-second-hunk: shown rows plus one tail note.
        assert_eq!(rendered_line_count(&multi, 3), 4);
        let _ = patch_element(&multi, &highlighted, 3);
    }

    /// MFST-63 step 1 decision gate: every required language must
    /// resolve to a plausible syntax in the combined set (syntect
    /// defaults plus the vendored extras below). Kept as lasting
    /// coverage: if a syntect upgrade or a vendored file regresses,
    /// highlighting falls back to plain and this test names the gap.
    ///
    /// Vendored extras in `assets/syntax/` (byte-identical copies):
    /// - `TypeScript.sublime-syntax`: converted from Microsoft's
    ///   official grammar by the bat project, vendored from
    ///   <https://github.com/sharkdp/bat/blob/master/assets/syntaxes/02_Extra/TypeScript.sublime-syntax>
    ///   (upstream: <https://github.com/Microsoft/TypeScript-Sublime-Plugin>).
    ///   SublimeHQ Packages carries no TypeScript syntax, and the
    ///   plugin ships `.tmLanguage` only, which syntect cannot parse.
    /// - `Kotlin.sublime-syntax`: <https://github.com/guille/sublime-kotlin>
    ///   (public domain; SublimeHQ Packages carries no Kotlin syntax).
    /// - `TOML.sublime-syntax`: <https://github.com/sublimehq/Packages/blob/master/TOML/TOML.sublime-syntax>
    ///   (same repo the defaults come from; the embedded defaults
    ///   predate its addition).
    ///
    /// `.mjs`/`.cjs` deliberately have no vendored file: they are the
    /// same language as the built-in JavaScript syntax, so the
    /// selector (step 2) maps them to the `js` lookup. Upstream's
    /// refreshed `JavaScript.sublime-syntax` was NOT vendored: it
    /// uses `extends: JavaScript (Plain).sublime-syntax`, which
    /// syntect 5.x silently ignores, so loading it would highlight
    /// nothing rather than fail loudly.
    #[test]
    fn syntax_set_covers_required_languages() {
        use syntect::parsing::{SyntaxDefinition, SyntaxSet};

        // Mirrors the production construction (step 2 promotes this
        // behind a `OnceLock`): defaults first so the vendored extras
        // only fill gaps, never shadow a built-in.
        let mut builder = SyntaxSet::load_defaults_nonewlines().into_builder();
        for file in [
            include_str!("../assets/syntax/TypeScript.sublime-syntax"),
            include_str!("../assets/syntax/Kotlin.sublime-syntax"),
            include_str!("../assets/syntax/TOML.sublime-syntax"),
        ] {
            builder.add(
                SyntaxDefinition::load_from_str(file, true, None)
                    .expect("vendored syntax must parse"),
            );
        }
        let syntaxes: SyntaxSet = builder.build();
        // Sanity: the builder round-trip keeps the defaults (the
        // vendored extras must fill gaps, never replace built-ins).
        assert_eq!(
            syntaxes
                .find_syntax_by_extension("rs")
                .map(|syntax| syntax.name.as_str()),
            Some("Rust")
        );

        // (probe extension, plausible syntax names). `mjs`/`cjs` go
        // through the selector's alias rule to the `js` lookup.
        let required: &[(&str, &[&str])] = &[
            ("rs", &["Rust"]),
            ("ts", &["TypeScript"]),
            ("js", &["JavaScript"]),
            ("kt", &["Kotlin"]),
            ("kts", &["Kotlin"]),
            ("py", &["Python"]),
            ("toml", &["TOML"]),
            ("json", &["JSON"]),
            ("md", &["Markdown"]),
        ];
        for (extension, names) in required {
            let found = syntaxes.find_syntax_by_extension(extension);
            // Report the actual resolution; --nocapture shows it.
            println!(
                "extension .{extension} -> {}",
                found.map_or(String::from("<none>"), |syntax| syntax.name.clone())
            );
            let syntax = found
                .unwrap_or_else(|| panic!("no syntax for extension .{extension}"));
            assert!(
                names.iter().any(|name| syntax.name.contains(name)),
                "extension .{extension} resolved to {:?}, expected one of {names:?}",
                syntax.name
            );
        }
        for alias in ["mjs", "cjs"] {
            let syntax = syntaxes
                .find_syntax_by_extension("js")
                .expect("built-in JavaScript must exist for .mjs/.cjs alias");
            println!("extension .{alias} -> {} (via js alias)", syntax.name);
            assert!(syntax.name.contains("JavaScript"));
        }
    }

    /// Builds a one-file patch document: every line is an added line,
    /// so highlighting also exercises diff-prefix stripping.
    fn highlight_sample(path: &str, texts: &[&str]) -> HighlightedFile {
        let highlighted = highlight_document(&PatchDocument {
            files: vec![PatchFile {
                path: String::from(path),
                hunks: vec![hunk(
                    1,
                    1,
                    texts
                        .iter()
                        .map(|text| line(PatchLineKind::Added, &format!("+{text}")))
                        .collect(),
                )],
            }],
        });
        assert_eq!(highlighted.files.len(), 1);
        highlighted.files.into_iter().next().expect("one file")
    }

    /// Spans of the file's single hunk, in document order.
    fn spans_of(file: &HighlightedFile) -> Vec<&Vec<(String, SyntaxCategory)>> {
        assert_eq!(file.hunks.len(), 1);
        file.hunks[0].lines.iter().map(|line| &line.spans).collect()
    }

    /// Asserts every line's spans join back to the stripped text:
    /// highlighting never drops or invents characters.
    fn assert_lossless(file: &HighlightedFile, stripped: &[&str]) {
        let lines: Vec<String> = file.hunks[0]
            .lines
            .iter()
            .map(|line| line.spans.iter().map(|(text, _)| text.as_str()).collect())
            .collect();
        let expected: Vec<String> = stripped.iter().map(|text| text.to_string()).collect();
        assert_eq!(lines, expected);
    }

    /// Asserts one line's spans read as `(text, category)` pairs.
    fn assert_spans(spans: &[(String, SyntaxCategory)], expected: &[(&str, SyntaxCategory)]) {
        let actual: Vec<(&str, SyntaxCategory)> = spans
            .iter()
            .map(|(text, category)| (text.as_str(), *category))
            .collect();
        let expected: Vec<(&str, SyntaxCategory)> =
            expected.iter().map(|(text, category)| (*text, *category)).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn rust_highlights_keywords_strings_comments_and_numbers() {
        let file = highlight_sample(
            "src/main.rs",
            &["fn main() {", "let x = 42;", "// comment", "let s = \"hi\";"],
        );
        assert!(file.language.as_deref().is_some_and(|name| name.contains("Rust")));
        let spans = spans_of(&file);
        assert_lossless(&file, &["fn main() {", "let x = 42;", "// comment", "let s = \"hi\";"]);
        // `fn` is a declaration keyword (`storage.type.function`).
        assert_eq!(spans[0][0], (String::from("fn"), SyntaxCategory::Keyword));
        assert!(spans[0].contains(&(String::from("main"), SyntaxCategory::Function)));
        // `let` scopes as storage: a type-colored declaration.
        assert!(spans[1].contains(&(String::from("let"), SyntaxCategory::Type)));
        assert!(spans[1].contains(&(String::from("42"), SyntaxCategory::Number)));
        assert_spans(spans[2], &[("// comment", SyntaxCategory::Comment)]);
        assert!(spans[3].contains(&(String::from("\"hi\""), SyntaxCategory::String)));
    }

    #[test]
    fn python_highlights_definitions_strings_and_comments() {
        let file = highlight_sample(
            "x.py",
            &["def f():", "x = \"hi\"  # comment", "return 42"],
        );
        assert!(file.language.as_deref().is_some_and(|name| name.contains("Python")));
        let spans = spans_of(&file);
        assert_lossless(&file, &["def f():", "x = \"hi\"  # comment", "return 42"]);
        assert_eq!(spans[0][0], (String::from("def"), SyntaxCategory::Keyword));
        assert!(spans[0].contains(&(String::from("f"), SyntaxCategory::Function)));
        assert!(spans[1].iter().any(|(_, category)| *category == SyntaxCategory::String));
        assert!(spans[1].iter().any(|(_, category)| *category == SyntaxCategory::Comment));
        assert!(spans[2].contains(&(String::from("return"), SyntaxCategory::Keyword)));
        assert!(spans[2].contains(&(String::from("42"), SyntaxCategory::Number)));
    }

    #[test]
    fn typescript_highlights_declarations_types_and_strings() {
        let file = highlight_sample(
            "src/app.ts",
            &["function f(x: string): number {", "const s = \"hi\"; // c"],
        );
        assert!(file.language.as_deref().is_some_and(|name| name.contains("TypeScript")));
        let spans = spans_of(&file);
        assert_lossless(&file, &["function f(x: string): number {", "const s = \"hi\"; // c"]);
        assert_eq!(spans[0][0], (String::from("function"), SyntaxCategory::Keyword));
        assert!(spans[0].contains(&(String::from("f"), SyntaxCategory::Function)));
        // `string`/`number` annotations scope as primitive types.
        assert!(spans[0].contains(&(String::from("string"), SyntaxCategory::Type)));
        assert!(spans[0].contains(&(String::from("number"), SyntaxCategory::Type)));
        assert!(spans[1].iter().any(|(_, category)| *category == SyntaxCategory::String));
        assert!(spans[1].iter().any(|(_, category)| *category == SyntaxCategory::Comment));
    }

    #[test]
    fn javascript_highlights_functions_templates_and_comments() {
        let file = highlight_sample("src/app.js", &["function f() {", "const s = `hi`; // c"]);
        assert!(file.language.as_deref().is_some_and(|name| name.contains("JavaScript")));
        let spans = spans_of(&file);
        assert_lossless(&file, &["function f() {", "const s = `hi`; // c"]);
        assert_eq!(spans[0][0], (String::from("function"), SyntaxCategory::Keyword));
        assert!(spans[0].contains(&(String::from("f"), SyntaxCategory::Function)));
        assert!(spans[1].iter().any(|(_, category)| *category == SyntaxCategory::String));
        assert!(spans[1].iter().any(|(_, category)| *category == SyntaxCategory::Comment));
    }

    #[test]
    fn mjs_aliases_to_the_builtin_javascript_syntax() {
        let file = highlight_sample("src/app.mjs", &["function f() {", "let n = 42;"]);
        assert!(file.language.as_deref().is_some_and(|name| name.contains("JavaScript")));
        let spans = spans_of(&file);
        assert_eq!(spans[0][0], (String::from("function"), SyntaxCategory::Keyword));
        assert!(spans[1].contains(&(String::from("42"), SyntaxCategory::Number)));
    }

    #[test]
    fn kotlin_highlights_declarations_strings_and_comments() {
        let file = highlight_sample(
            "src/Main.kt",
            &["fun main() {", "val s = \"hi\" // c", "val n = 42"],
        );
        assert!(file.language.as_deref().is_some_and(|name| name.contains("Kotlin")));
        let spans = spans_of(&file);
        assert_lossless(&file, &["fun main() {", "val s = \"hi\" // c", "val n = 42"]);
        // `fun` scopes as a declaration keyword in the Kotlin grammar.
        assert_eq!(spans[0][0], (String::from("fun"), SyntaxCategory::Keyword));
        assert!(spans[0].contains(&(String::from("main"), SyntaxCategory::Function)));
        assert!(spans[1].iter().any(|(_, category)| *category == SyntaxCategory::String));
        assert!(spans[1].iter().any(|(_, category)| *category == SyntaxCategory::Comment));
        assert!(spans[2].contains(&(String::from("42"), SyntaxCategory::Number)));
    }

    #[test]
    fn toml_highlights_strings_comments_and_numbers() {
        let file = highlight_sample("a.toml", &["key = \"value\" # c", "n = 42"]);
        assert!(file.language.as_deref().is_some_and(|name| name.contains("TOML")));
        let spans = spans_of(&file);
        assert_lossless(&file, &["key = \"value\" # c", "n = 42"]);
        assert!(spans[0].iter().any(|(_, category)| *category == SyntaxCategory::String));
        assert!(spans[0].iter().any(|(_, category)| *category == SyntaxCategory::Comment));
        assert!(spans[1].contains(&(String::from("42"), SyntaxCategory::Number)));
    }

    #[test]
    fn json_highlights_strings_and_numbers() {
        let file = highlight_sample("a.json", &["{\"key\": \"value\", \"n\": 42}"]);
        assert!(file.language.as_deref().is_some_and(|name| name.contains("JSON")));
        let spans = spans_of(&file);
        assert_lossless(&file, &["{\"key\": \"value\", \"n\": 42}"]);
        assert!(spans[0].iter().any(|(_, category)| *category == SyntaxCategory::String));
        assert!(spans[0].contains(&(String::from("42"), SyntaxCategory::Number)));
    }

    #[test]
    fn markdown_highlights_headings_code_and_links() {
        // Markup scopes map onto the shared token categories:
        // headings read as types, raw code spans as strings, link
        // targets as constants. Emphasis without a color expression
        // (bold/italic) and ordinary paragraph text stay plain.
        let file = highlight_sample(
            "a.md",
            &[
                "# Heading",
                "`code` and **bold**",
                "plain text",
                "[t](https://example.com)",
            ],
        );
        assert!(file.language.as_deref().is_some_and(|name| name.contains("Markdown")));
        let spans = spans_of(&file);
        assert_lossless(
            &file,
            &[
                "# Heading",
                "`code` and **bold**",
                "plain text",
                "[t](https://example.com)",
            ],
        );
        // Heading marker and text all resolve outward to the heading
        // scope: every span reads as a type.
        assert!(!spans[0].is_empty());
        assert!(spans[0].iter().all(|(_, category)| *category == SyntaxCategory::Type));
        // Inline code reads as a string; bold has no color expression
        // and stays plain alongside its surrounding text.
        assert!(spans[1].iter().any(|(_, category)| *category == SyntaxCategory::String));
        assert!(spans[1].iter().any(|(_, category)| *category == SyntaxCategory::Plain));
        // Ordinary paragraph text stays one plain span.
        assert_spans(spans[2], &[("plain text", SyntaxCategory::Plain)]);
        // The link target reads as a constant; the surrounding
        // punctuation and description stay plain.
        assert!(spans[3].iter().any(|(_, category)| *category == SyntaxCategory::Constant));
        assert!(spans[3].iter().any(|(_, category)| *category == SyntaxCategory::Plain));
    }

    #[test]
    fn unknown_extension_stays_plain_with_diff_colors_intact() {
        let file = highlight_sample("a.xyz123", &["fn main() {", "let x = 42;"]);
        assert_eq!(file.language, None);
        let spans = spans_of(&file);
        assert_lossless(&file, &["fn main() {", "let x = 42;"]);
        for line_spans in spans {
            assert_eq!(line_spans.len(), 1);
            assert_eq!(line_spans[0].1, SyntaxCategory::Plain);
        }
    }

    #[test]
    fn header_lines_stay_plain_so_they_render_tertiary() {
        // Header metadata lines never enter the syntax pass, even in
        // a file with a known language: one plain span per line, so
        // the renderer falls back to the tertiary header tone.
        let highlighted = highlight_document(&PatchDocument {
            files: vec![PatchFile {
                path: String::from("src/main.rs"),
                hunks: vec![hunk(
                    1,
                    1,
                    vec![
                        line(PatchLineKind::Header, "diff --git a/src/main.rs"),
                        line(PatchLineKind::Header, "index abc..def 100644"),
                        line(PatchLineKind::Added, "+fn main() {}"),
                    ],
                )],
            }],
        });
        let file = highlighted.files.into_iter().next().expect("one file");
        let spans = spans_of(&file);
        assert_lossless(&file, &["diff --git a/src/main.rs", "index abc..def 100644", "fn main() {}"]);
        for line_spans in &spans[..2] {
            assert_eq!(line_spans.len(), 1);
            assert_eq!(line_spans[0].1, SyntaxCategory::Plain);
        }
        // Code lines in the same file still highlight.
        assert!(spans[2].iter().any(|(_, category)| *category != SyntaxCategory::Plain));
    }

    /// Joins one [`highlight_code`] row back to its source line.
    fn joined(row: &[(String, SyntaxCategory)]) -> String {
        row.iter().map(|(text, _)| text.as_str()).collect()
    }

    #[test]
    fn code_python_highlights_keywords_strings_and_comments() {
        let rows = highlight_code(Some("python"), "def f():\nx = \"hi\"  # comment\n");
        assert_eq!(rows.len(), 3);
        // Trailing newline sheds to one final empty plain row.
        assert_eq!(joined(&rows[0]), "def f():");
        assert_eq!(joined(&rows[1]), "x = \"hi\"  # comment");
        assert_eq!(rows[0][0], (String::from("def"), SyntaxCategory::Keyword));
        assert!(rows[0].contains(&(String::from("f"), SyntaxCategory::Function)));
        assert!(rows[1].iter().any(|(_, category)| *category == SyntaxCategory::String));
        assert!(rows[1].iter().any(|(_, category)| *category == SyntaxCategory::Comment));
    }

    #[test]
    fn code_rust_highlights_keywords_and_numbers() {
        let rows = highlight_code(Some("rust"), "fn main() {\nlet x = 42;\n}");
        assert_eq!(rows.len(), 3);
        assert_eq!(joined(&rows[0]), "fn main() {");
        assert_eq!(rows[0][0], (String::from("fn"), SyntaxCategory::Keyword));
        assert!(rows[1].contains(&(String::from("42"), SyntaxCategory::Number)));
    }

    #[test]
    fn code_unknown_and_missing_languages_stay_plain() {
        for language in [None, Some(""), Some("   "), Some("xyz123")] {
            let rows = highlight_code(language, "fn main() {\nlet x = 42;\n}\n");
            assert_eq!(rows.len(), 4);
            assert_eq!(joined(&rows[0]), "fn main() {");
            assert_eq!(joined(&rows[1]), "let x = 42;");
            assert_eq!(joined(&rows[2]), "}");
            assert_eq!(joined(&rows[3]), "");
            for row in &rows {
                assert_eq!(row.len(), 1);
                assert_eq!(row[0].1, SyntaxCategory::Plain);
            }
        }
    }

    #[test]
    fn code_language_matching_ignores_case_and_accepts_extensions() {
        // `Python` matches by syntax name case-insensitively; `rs`
        // matches by extension.
        let by_name = highlight_code(Some("Python"), "def f():\nreturn 42\n");
        assert_eq!(by_name[0][0], (String::from("def"), SyntaxCategory::Keyword));
        assert!(by_name[1].contains(&(String::from("42"), SyntaxCategory::Number)));
        let by_extension = highlight_code(Some("rs"), "fn main() {\nlet x = 42;\n}");
        assert_eq!(by_extension[0][0], (String::from("fn"), SyntaxCategory::Keyword));
        assert!(by_extension[1].contains(&(String::from("42"), SyntaxCategory::Number)));
        // Whitespace around the fence token is trimmed.
        let padded = highlight_code(Some("  py  "), "def f():\n");
        assert_eq!(padded[0][0], (String::from("def"), SyntaxCategory::Keyword));
    }

    #[test]
    fn code_state_carries_across_lines() {
        // A multi-line string keeps later lines out of the plain
        // category: one sequential ParseState pass over all lines.
        let rows = highlight_code(Some("python"), "x = \"\"\"\nhello\n\"\"\"\ny = 1\n");
        assert_eq!(rows.len(), 5);
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(joined(row), ["x = \"\"\"", "hello", "\"\"\"", "y = 1", ""][index]);
        }
        assert!(rows[1].iter().any(|(_, category)| *category == SyntaxCategory::String));
        // The lone closing delimiter scopes plain in this grammar,
        // but it still ends the carried string context: the line
        // after the string highlights as code again.
        assert_spans(&rows[2], &[("\"\"\"", SyntaxCategory::Plain)]);
        assert!(rows[3].contains(&(String::from("1"), SyntaxCategory::Number)));
    }

    #[test]
    fn code_empty_input_is_one_plain_row() {
        let rows = highlight_code(Some("python"), "");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], vec![(String::new(), SyntaxCategory::Plain)]);
    }

    #[test]
    fn category_colors_come_from_theme_tokens_only() {
        use crate::theme::{
            SYNTAX_COMMENT, SYNTAX_CONSTANT, SYNTAX_FUNCTION, SYNTAX_KEYWORD, SYNTAX_NUMBER,
            SYNTAX_STRING, SYNTAX_TYPE,
        };
        assert_eq!(category_color(SyntaxCategory::Keyword), Some(SYNTAX_KEYWORD));
        assert_eq!(category_color(SyntaxCategory::String), Some(SYNTAX_STRING));
        assert_eq!(category_color(SyntaxCategory::Comment), Some(SYNTAX_COMMENT));
        assert_eq!(category_color(SyntaxCategory::Type), Some(SYNTAX_TYPE));
        assert_eq!(category_color(SyntaxCategory::Function), Some(SYNTAX_FUNCTION));
        assert_eq!(category_color(SyntaxCategory::Number), Some(SYNTAX_NUMBER));
        assert_eq!(category_color(SyntaxCategory::Constant), Some(SYNTAX_CONSTANT));
        // Plain carries no token color: the caller falls back to the
        // per-kind diff line color.
        assert_eq!(category_color(SyntaxCategory::Plain), None);
    }
}
