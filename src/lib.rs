//! A reusable **block-swap live-preview** editor for Dioxus (Markdown by default).
//!
//! The document is shown as a column of styled markdown *blocks*. The block the
//! user is editing turns into a raw-markdown `<textarea>`; clicking away (or
//! pressing Esc) re-renders it. Because every editable element is an ordinary
//! controlled `<textarea>`, there is no `contenteditable`-vs-virtual-DOM caret
//! conflict — the price is that inline formatting does not render *within* the
//! line currently being edited (that is the `contenteditable`-only trick).
//!
//! The crate is deliberately app-agnostic: it knows nothing about any particular
//! markdown dialect, sanitizer, or embed syntax. The caller injects
//!
//! * `render_block` — turns one block's markdown into an [`Element`], and
//! * `is_atomic` — marks a line (e.g. a custom embed) that must stand alone.
//!
//! so the host application reuses its own rendering pipeline unchanged.

use dioxus::prelude::*;
use riparion_dnd::{DragDropArea, DragDropEvent, Draggable, DropList, DEFAULT_STYLE};

mod autocomplete;
pub use autocomplete::{
    apply_completion, find_trigger, use_autocomplete, Autocomplete, CompletionItem,
    CompletionPopup, KeyOutcome, TriggerMatch,
};

/// One contiguous slice of the source document.
///
/// **Invariant:** concatenating every block's [`text`](Block::text) in document
/// order reproduces the original source byte-for-byte. Splitting is therefore
/// lossless: [`join_blocks`]`(&`[`split_blocks`]`(s, p)) == s` for every `s`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// The exact source slice, including its own trailing newline(s) and any
    /// blank-line separator that follows it. Never trimmed — trimming would
    /// break the round-trip invariant.
    pub text: String,
    /// True when this block is a single `is_atomic` line (e.g. an embed) held
    /// apart so the user edits just that one line.
    pub atomic: bool,
    /// True when this block is the document's leading YAML frontmatter
    /// (`---` … `---` starting at byte 0). Always block 0 when present; kept
    /// whole by [`split_blocks`] and pinned (not draggable) by [`BlockEditor`].
    pub frontmatter: bool,
}

/// Concatenate blocks back into a single source string. The exact inverse of
/// [`split_blocks`].
pub fn join_blocks(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        out.push_str(&b.text);
    }
    out
}

/// Split `md` into a lossless sequence of [`Block`]s.
///
/// Boundaries are chosen so the result both round-trips exactly and edits
/// cleanly:
///
/// * **YAML frontmatter** (`---` … `---` starting at byte 0) is one block,
///   flagged [`frontmatter`](Block::frontmatter) — blank lines, fences and
///   `is_atomic` lines inside it never split it.
/// * **Fenced code** (` ``` ` / `~~~`) is never split mid-fence — blank lines
///   inside a fence stay with it.
/// * A line for which `is_atomic` returns true becomes its own block.
/// * Otherwise blocks break on blank lines; a blank line attaches to the block
///   it follows, so separators are never lost.
///
/// Leading or whitespace-only runs are preserved as their own blocks (rendered
/// as a thin clickable spacer by [`BlockEditor`]) rather than discarded.
pub fn split_blocks(md: &str, is_atomic: impl Fn(&str) -> bool) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut cur = String::new();
    // `Some((fence_char, len))` while inside a fenced code block.
    let mut fence: Option<(char, usize)> = None;
    // The current block has absorbed a trailing blank line; the next non-blank
    // line therefore starts a fresh block.
    let mut closed = false;

    let flush = |cur: &mut String, blocks: &mut Vec<Block>| {
        if !cur.is_empty() {
            blocks.push(Block {
                text: std::mem::take(cur),
                atomic: false,
                frontmatter: false,
            });
        }
    };

    // Frontmatter is the outermost rule: consume it whole before the line loop
    // so neither the fence state nor `is_atomic` can split its interior. Only
    // a run starting at byte 0 qualifies — a `---` later in the document falls
    // through to the ordinary rules below.
    let md = match frontmatter_len(md) {
        Some(mut len) => {
            // The blank separator after the closing delimiter attaches to the
            // frontmatter block — same rule as any block's trailing blank line —
            // so no stray spacer block appears between it and the body.
            for line in md[len..].split_inclusive('\n') {
                if !trim_line_end(line).trim().is_empty() {
                    break;
                }
                len += line.len();
            }
            blocks.push(Block {
                text: md[..len].to_string(),
                atomic: false,
                frontmatter: true,
            });
            &md[len..]
        }
        None => md,
    };

    // `split_inclusive` keeps each line's trailing '\n', so concatenating the
    // pieces reproduces `md` exactly (the final line keeps its missing newline).
    for line in md.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let content = content.strip_suffix('\r').unwrap_or(content);

        // Inside a fence: swallow everything (including blanks) until the close.
        if let Some((fch, flen)) = fence {
            cur.push_str(line);
            if is_closing_fence(content, fch, flen) {
                fence = None;
            }
            continue;
        }

        if content.trim().is_empty() {
            // Blank line: keep it with the current block and arm a break.
            cur.push_str(line);
            closed = true;
            continue;
        }

        // Only open a fence at a block boundary — the start of the document or
        // right after a blank line. A ``` that appears mid-paragraph (no preceding
        // blank line) is treated as ordinary text rather than swallowing the rest
        // of the block as code.
        if closed || cur.is_empty() {
            if let Some(open) = open_fence(content) {
                if closed {
                    flush(&mut cur, &mut blocks);
                    closed = false;
                }
                cur.push_str(line);
                fence = Some(open);
                continue;
            }
        }

        if is_atomic(content) {
            flush(&mut cur, &mut blocks);
            blocks.push(Block {
                text: line.to_string(),
                atomic: true,
                frontmatter: false,
            });
            closed = false;
            continue;
        }

        // Ordinary content line.
        if closed {
            flush(&mut cur, &mut blocks);
            closed = false;
        }
        cur.push_str(line);
    }

    flush(&mut cur, &mut blocks);
    blocks
}

/// If `content` opens a code fence, return its fence char and run length.
///
/// Follows CommonMark's indentation rule: a fence opener may be indented at most
/// three spaces. Four or more spaces (or a leading tab) make the line an *indented*
/// code block instead, so it is not treated as a fence.
fn open_fence(content: &str) -> Option<(char, usize)> {
    // Leading-whitespace width: a tab counts as four columns, a space as one.
    let indent: usize = content
        .chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum();
    if indent >= 4 {
        return None;
    }
    let t = content.trim_start();
    let ch = t.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let len = t.chars().take_while(|&c| c == ch).count();
    (len >= 3).then_some((ch, len))
}

/// A closing fence is a line of only `fch` (no info string) at least as long as
/// the opener.
fn is_closing_fence(content: &str, fch: char, flen: usize) -> bool {
    let t = content.trim();
    let len = t.chars().take_while(|&c| c == fch).count();
    len >= flen && t.chars().all(|c| c == fch)
}

/// If `md` opens with YAML frontmatter, return the byte length of the whole
/// run — opening `---`, interior, and closing delimiter line (including its
/// trailing newline, when present).
///
/// The opener must be exactly `---` on the very first line (byte offset 0); a
/// `---` anywhere later in a document is a thematic break, not frontmatter.
/// The run closes at the next line that is exactly `---` or `...` (YAML's
/// document-end marker). An unclosed opener returns `None`, so a lone leading
/// `---` keeps its ordinary thematic-break meaning instead of swallowing the
/// document. Lines are compared with any trailing `\r` stripped, so CRLF
/// sources work; the returned length is in bytes of the original `md`.
pub fn frontmatter_len(md: &str) -> Option<usize> {
    let mut lines = md.split_inclusive('\n');
    let first = lines.next()?;
    if trim_line_end(first) != "---" {
        return None;
    }
    let mut len = first.len();
    for line in lines {
        len += line.len();
        let content = trim_line_end(line);
        if content == "---" || content == "..." {
            return Some(len);
        }
    }
    None
}

/// Strip a line's trailing `\n` / `\r\n`, if any.
fn trim_line_end(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

/// Join blocks with canonical single-blank-line separators, dropping empty /
/// whitespace-only blocks. Only the separators *between* blocks are touched —
/// each block's own content (leading indentation, soft line breaks) is preserved.
/// Used after a *structural* edit (insert, reorder) so adjacent blocks can't merge
/// or pile up stray blank lines.
pub fn rejoin_normalized(blocks: &[Block]) -> String {
    // A leading frontmatter block is kept verbatim (one trailing newline) so
    // normalization can never restructure the YAML; the body that follows is
    // normalized as usual and separated by the canonical blank line.
    if let Some((first, rest)) = blocks.split_first() {
        if first.frontmatter {
            let head = first.text.trim_end_matches(['\n', '\r']);
            let body = rejoin_normalized(rest);
            return if body.is_empty() {
                format!("{head}\n")
            } else {
                format!("{head}\n\n{body}")
            };
        }
    }
    let parts: Vec<&str> = blocks
        .iter()
        .map(|b| b.text.trim_end_matches(['\n', '\r']))
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("{}\n", parts.join("\n\n"))
    }
}

/// Re-split `md` and rejoin it with [`rejoin_normalized`]. The canonical form of
/// a body after a structural edit.
pub fn normalize_body(md: &str, is_atomic: impl Fn(&str) -> bool) -> String {
    rejoin_normalized(&split_blocks(md, is_atomic))
}

/// Ensure `s` ends with a paragraph break (a blank line) so whatever follows it
/// starts its own paragraph. No-op when `s` is empty or already blank-terminated.
fn ensure_paragraph_break(s: &mut String) {
    if !s.is_empty() && !s.ends_with("\n\n") {
        if s.ends_with('\n') {
            s.push('\n');
        } else {
            s.push_str("\n\n");
        }
    }
}

/// Remove block `i` and return the normalized body (the [`rejoin_normalized`]
/// form). Out-of-range `i` is a no-op. The pure core of [`BlockChrome::delete`].
pub fn delete_and_normalize(blocks: &[Block], i: usize) -> String {
    let mut v = blocks.to_vec();
    if i < v.len() {
        v.remove(i);
    }
    rejoin_normalized(&v)
}

/// Insert `text` immediately after block `i` and return the normalized body.
/// The text is re-split with [`split_blocks`], so a multi-paragraph paste
/// becomes proper separate blocks; whitespace-only text is dropped by
/// normalization. The pure core of [`BlockChrome::insert_after`].
pub fn insert_after_and_normalize(
    blocks: &[Block],
    i: usize,
    text: &str,
    is_atomic: impl Fn(&str) -> bool,
) -> String {
    let mut v = blocks.to_vec();
    let at = (i + 1).min(v.len());
    if at > 0 {
        ensure_paragraph_break(&mut v[at - 1].text);
    }
    // Whitespace-only pieces are blank separators, not content — skip them
    // (`rejoin_normalized` only drops fully-empty blocks, not indented ones).
    let inserted = split_blocks(text, is_atomic)
        .into_iter()
        .filter(|b| !b.text.trim().is_empty());
    for (k, b) in inserted.enumerate() {
        v.insert(at + k, b);
    }
    rejoin_normalized(&v)
}

/// Stable, content-derived render keys for a block list, so Dioxus diffs blocks by
/// identity rather than position — index keys make it reuse the wrong DOM node when
/// blocks are reordered or inserted. Identical-content blocks are disambiguated by
/// their occurrence order; every key contains a `#`, so callers can mint a
/// collision-free constant key (e.g. for the active textarea) by omitting one.
fn block_keys(blocks: &[Block]) -> Vec<String> {
    use std::collections::HashMap;
    let mut seen: HashMap<&str, usize> = HashMap::new();
    blocks
        .iter()
        .map(|b| {
            let n = seen.entry(b.text.as_str()).or_insert(0);
            let key = format!("{}#{n}", b.text);
            *n += 1;
            key
        })
        .collect()
}

/// Convert a UTF-16 code-unit index (what the browser reports for textarea
/// selection) into a Rust byte index, so slicing the value is correct for
/// multi-byte text. Clamps to the string length.
#[cfg(feature = "web")]
fn utf16_to_byte(s: &str, utf16_idx: usize) -> usize {
    let mut units = 0usize;
    for (byte, ch) in s.char_indices() {
        if units >= utf16_idx {
            return byte;
        }
        units += ch.len_utf16();
    }
    s.len()
}

/// Count UTF-16 code units in `s[..byte_idx]` (the inverse of [`utf16_to_byte`]),
/// for setting a textarea selection — the browser addresses it in UTF-16.
#[cfg(feature = "web")]
fn byte_to_utf16(s: &str, byte_idx: usize) -> u32 {
    s[..byte_idx].chars().map(|c| c.len_utf16() as u32).sum()
}

/// Byte offset of the `col`-th char within `line`, clamped to its end. Used to
/// keep the caret's column when arrow-navigating between blocks.
#[cfg(feature = "web")]
fn col_to_byte(line: &str, col: usize) -> usize {
    line.char_indices()
        .nth(col)
        .map(|(b, _)| b)
        .unwrap_or(line.len())
}

/// The textarea that fired key event `e`, plus its value and current selection
/// (`selectionStart`/`selectionEnd`, in UTF-16 code units). `None` off web.
#[cfg(feature = "web")]
fn textarea_state(
    e: &KeyboardEvent,
) -> Option<(web_sys::HtmlTextAreaElement, String, usize, usize)> {
    use dioxus::web::WebEventExt;
    use wasm_bindgen::JsCast;
    let we: web_sys::KeyboardEvent = e.data().try_as_web_event()?;
    let ta = we
        .target()?
        .dyn_into::<web_sys::HtmlTextAreaElement>()
        .ok()?;
    let value = ta.value();
    let start = ta.selection_start().ok().flatten()? as usize;
    let end = ta.selection_end().ok().flatten()? as usize;
    Some((ta, value, start, end))
}

/// Read `(value, caret_byte_offset)` from the textarea that fired key event `e`.
#[cfg(feature = "web")]
fn textarea_caret(e: &KeyboardEvent) -> Option<(String, usize)> {
    let (_, value, start, _) = textarea_state(e)?;
    let byte = utf16_to_byte(&value, start);
    Some((value, byte))
}

/// An inline-format action resolved from a key press: wrap the current selection
/// (or caret) with `open`…`close`. Replaces the old positional
/// `(String, String, bool, bool)` tuple so each field reads at the call site.
#[cfg(feature = "web")]
struct WrapSpec {
    /// Text inserted before the selection (e.g. `**`, `[`).
    open: String,
    /// Text inserted after the selection (e.g. `**`, `](url)`).
    close: String,
    /// After wrapping, select the `url` placeholder inside `close` (link insert)
    /// instead of keeping the selection on the wrapped text.
    select_url: bool,
    /// Only act when there is a non-empty selection. Typed `* _ ~ \`` require one;
    /// the Cmd/Ctrl shortcuts also fire on an empty caret.
    require_selection: bool,
}

/// Per-block chrome handed to a host's `wrap_block` callback so it can wrap an
/// inactive, rendered block in app-specific UI (e.g. a right-click context
/// menu). The crate stays widget-free: it exposes only the block's identity,
/// its source, the already-rendered child, and the structural ops to act on it.
#[derive(Clone)]
pub struct BlockChrome {
    /// This block's position in the current view (0-based).
    pub index: usize,
    /// This block's exact markdown source (the same string `render_block` saw).
    pub text: String,
    /// The rendered, clickable block content to wrap. The returned Element must
    /// include it, or the block disappears.
    pub children: Element,
    /// Delete block `i` (pass [`BlockChrome::index`]).
    pub delete: Callback<usize>,
    /// Insert `text` as new block(s) immediately after block `i`
    /// (pass `(index, text)`). Multi-paragraph text becomes multiple blocks.
    pub insert_after: Callback<(usize, String)>,
}

/// The block-swap live-preview editor.
///
/// `body` is the single source of truth (canonical markdown); the editor splices
/// edits into it in place, so the host's save path is unchanged. See the
/// [crate docs](crate) for the injected-behavior contract.
#[component]
pub fn BlockEditor(
    /// Canonical markdown source. Mutated in place as the user edits.
    body: Signal<String>,
    /// Render one block's markdown to an [`Element`] (the host's pipeline).
    render_block: Callback<String, Element>,
    /// Lines for which this returns true become their own atomic block.
    is_atomic: Option<Callback<String, bool>>,
    /// Class for an inactive, rendered block wrapper.
    #[props(default)]
    block_class: String,
    /// Class for the active-block `<textarea>`.
    #[props(default)]
    textarea_class: String,
    /// Optional inline-autocomplete provider: given the query typed after `[[/`,
    /// return the candidates to offer. `None` (the default) disables
    /// autocomplete entirely — fully back-compatible. See [`use_autocomplete`].
    #[props(default)]
    complete: Option<Callback<String, Vec<CompletionItem>>>,
    /// Class for the autocomplete menu container (see [`CompletionPopup`]).
    #[props(default)]
    completion_menu_class: String,
    /// Class for each autocomplete item row.
    #[props(default)]
    completion_item_class: String,
    /// Class added to the highlighted autocomplete item.
    #[props(default)]
    completion_item_active_class: String,
    /// Optional per-block wrapper: given a [`BlockChrome`] (block index, source,
    /// the rendered child, and structural-op callbacks), return the Element to
    /// render in that block's place — e.g. wrap it in a right-click context
    /// menu. Applies to inactive, rendered blocks only (never the active
    /// textarea). `None` (the default) renders blocks bare — fully
    /// back-compatible.
    #[props(default)]
    wrap_block: Option<Callback<BlockChrome, Element>>,
) -> Element {
    let mut body = body;
    // The single autocomplete behavior source, shared with any host textarea.
    let ac = use_autocomplete(complete);
    // Index of the block currently being edited as raw markdown, if any.
    let mut active = use_signal(|| Option::<usize>::None);
    // Snapshot taken when a block is activated. While active we render from this
    // snapshot and never re-split, so the active index and the textarea's caret
    // stay stable no matter what the user types (including blank lines).
    let mut frozen = use_signal(Vec::<Block>::new);
    // Set when we *programmatically* move focus to another block (double-Enter,
    // arrow nav). Removing the old textarea fires a `blur`; without this guard that
    // blur would immediately deactivate the editor and normalize away the new block.
    let mut moving = use_signal(|| false);
    // Caret offset (UTF-16) to apply once the next-activated block's textarea
    // mounts — lets cross-block arrow nav land the caret at the right column.
    // Web-only: it's read/written solely from the web-gated key + mount handlers.
    #[cfg(feature = "web")]
    let mut pending_caret = use_signal(|| Option::<u32>::None);

    let atomic = move |line: &str| match is_atomic {
        Some(cb) => cb.call(line.to_string()),
        None => false,
    };

    // Inactive view: re-split `body` reactively.
    let blocks = use_memo(move || split_blocks(&body(), atomic));

    let activate = use_callback(move |i: usize| {
        frozen.set(split_blocks(&body(), atomic));
        active.set(Some(i));
    });

    // Insert a fresh, empty block right *below the one being edited* (or at the
    // end if nothing is active) and start editing it. We force the preceding
    // block to end in a blank line so the new block is its own paragraph; the
    // following block's separation is restored by normalize-on-blur. The new
    // empty block isn't produced by `split_blocks`, so we build the snapshot by
    // hand and render from it until the next blur re-splits.
    let add_block = use_callback(move |_: ()| {
        // While a block is active, the frozen snapshot — not a fresh re-split of
        // body() — is the source of truth (H1). Re-splitting could fragment the
        // active block on its own internal blank lines and drop the new block in
        // the wrong place.
        let mut snap = match active() {
            Some(_) => frozen(),
            None => split_blocks(&body(), atomic),
        };
        let at = match active() {
            Some(i) => (i + 1).min(snap.len()),
            None => snap.len(),
        };
        if at > 0 {
            ensure_paragraph_break(&mut snap[at - 1].text);
        }
        snap.insert(
            at,
            Block {
                text: String::new(),
                atomic: false,
                frontmatter: false,
            },
        );
        active.set(Some(at));
        body.set(join_blocks(&snap));
        frozen.set(snap);
    });

    // Delete block `i` (exposed to the host through [`BlockChrome`]). Like
    // `add_block`, while a block is active the frozen snapshot — not a fresh
    // re-split — is the source of truth (H1), so we mutate it and keep `body`
    // in sync with `join_blocks`; normalize-on-blur canonicalizes spacing
    // later. When idle, this is a structural edit like `on_drop`: re-split and
    // rejoin normalized.
    let delete_block = use_callback(move |i: usize| {
        if active().is_some() {
            let mut snap = frozen();
            if i >= snap.len() {
                return;
            }
            snap.remove(i);
            // Removing a block before the active one shifts it left. The active
            // block itself is never wrapped, but handle it defensively.
            match active() {
                Some(a) if a == i => active.set(None),
                Some(a) if a > i => active.set(Some(a - 1)),
                _ => {}
            }
            body.set(join_blocks(&snap));
            frozen.set(snap);
        } else {
            let bs = split_blocks(&body(), atomic);
            if i >= bs.len() {
                return;
            }
            body.set(delete_and_normalize(&bs, i));
        }
    });

    // Insert `text` as new block(s) right below block `i` (exposed to the host
    // through [`BlockChrome`] — the "paste" op). Same frozen-vs-resplit duality
    // as `delete_block`. The text is re-split so a multi-paragraph paste lands
    // as proper separate blocks.
    let insert_block_after = use_callback(move |(i, text): (usize, String)| {
        if active().is_some() {
            let mut snap = frozen();
            let at = (i + 1).min(snap.len());
            if at > 0 {
                ensure_paragraph_break(&mut snap[at - 1].text);
            }
            let mut inserted = split_blocks(&text, atomic);
            // Keep the pasted block(s) separated from whatever follows.
            if let Some(last) = inserted.last_mut() {
                ensure_paragraph_break(&mut last.text);
            }
            let n = inserted.len();
            for (k, b) in inserted.into_iter().enumerate() {
                snap.insert(at + k, b);
            }
            // Inserting at or before the active block shifts it right.
            if let Some(a) = active() {
                if at <= a {
                    active.set(Some(a + n));
                }
            }
            body.set(join_blocks(&snap));
            frozen.set(snap);
        } else {
            let bs = split_blocks(&body(), atomic);
            body.set(insert_after_and_normalize(&bs, i, &text, atomic));
        }
    });

    // Hand an inactive block row to the host's `wrap_block`, if any.
    let render_row = move |i: usize, text: String, child: Element| -> Element {
        match wrap_block {
            Some(cb) => cb.call(BlockChrome {
                index: i,
                text,
                children: child,
                delete: delete_block,
                insert_after: insert_block_after,
            }),
            None => child,
        }
    };

    // Drag-and-drop block reordering, via the riparion-dnd primitives. The drop event
    // carries source/target slots; we move the block and rejoin with canonical
    // separators (reorder is a structural edit, so spacing is normalized).
    let on_drop = use_callback(move |evt: DragDropEvent<usize>| {
        let mut bs = split_blocks(&body(), atomic);
        // A leading frontmatter block is pinned: it renders outside the drag
        // area so its slot never exists, but guard defensively against any
        // drop that would move it or land something above it.
        let base = bs.first().map_or(0, |b| b.frontmatter as usize);
        if evt.from_slot < base || evt.to_slot < base {
            return;
        }
        if evt.from_slot >= bs.len() {
            return;
        }
        let item = bs.remove(evt.from_slot);
        // Removing an earlier item shifts the target left by one.
        let to = if evt.from_slot < evt.to_slot {
            evt.to_slot - 1
        } else {
            evt.to_slot
        };
        let to = to.min(bs.len());
        bs.insert(to, item);
        body.set(rejoin_normalized(&bs));
    });

    // Write block `i`'s new text into the frozen snapshot and resync `body` from
    // it. The single edit-while-active commit path (D2): used by typing into the
    // textarea and by the inline-format shortcuts.
    let mut commit_block = move |i: usize, text: String| {
        {
            let mut f = frozen.write();
            if let Some(b) = f.get_mut(i) {
                b.text = text;
            }
        }
        body.set(join_blocks(&frozen.read()));
    };

    // The big per-key behavior, split out of the textarea's `onkeydown` (K1) into
    // one closure per concern. All three are web-only: they read the live caret /
    // selection, which the server never does interactively.

    // Double-Enter (Notion-style): a plain Enter on a blank line ends this block and
    // opens a fresh one below, moving the caret to its start. A single Enter still
    // inserts a newline, so lists/code type normally.
    #[cfg(feature = "web")]
    let mut handle_enter = move |i: usize, e: &KeyboardEvent| {
        let Some((value, caret)) = textarea_caret(e) else {
            return;
        };
        let line_start = value[..caret].rfind('\n').map(|n| n + 1).unwrap_or(0);
        let line_end = value[caret..]
            .find('\n')
            .map(|n| caret + n)
            .unwrap_or(value.len());
        if !value[line_start..line_end].trim().is_empty() {
            return; // Not a blank line — let the browser insert a newline.
        }
        e.prevent_default();
        // Head keeps everything up to the blank line and ends in a paragraph break;
        // the rest becomes a new block the caret enters.
        let mut head = value[..line_start].to_string();
        ensure_paragraph_break(&mut head);
        let tail = value[line_end..]
            .strip_prefix('\n')
            .unwrap_or(&value[line_end..])
            .to_string();
        {
            let mut f = frozen.write();
            if i < f.len() {
                f[i].text = head;
                f.insert(
                    i + 1,
                    Block {
                        text: tail,
                        atomic: false,
                        frontmatter: false,
                    },
                );
            }
        }
        body.set(join_blocks(&frozen.read()));
        // The blur from removing this textarea must not deactivate us — we're
        // moving focus into the new block, caret at its start (L2).
        moving.set(true);
        pending_caret.set(Some(0));
        active.set(Some(i + 1));
    };

    // Cross-block arrow nav: Up/Left off the top/start of a block move into the
    // previous one, Down/Right off the bottom/end into the next — preserving the
    // caret column. Mid-block, the browser's default applies.
    #[cfg(feature = "web")]
    let mut handle_arrow_nav = move |i: usize, e: &KeyboardEvent| {
        let Some((value, caret)) = textarea_caret(e) else {
            return;
        };
        let len = value.len();
        let key = e.key();
        let line_start = value[..caret].rfind('\n').map(|n| n + 1).unwrap_or(0);
        let column = value[line_start..caret].chars().count();
        let on_first_line = !value[..caret].contains('\n');
        let on_last_line = !value[caret..].contains('\n');
        let go_prev =
            (key == Key::ArrowUp && on_first_line) || (key == Key::ArrowLeft && caret == 0);
        let go_next =
            (key == Key::ArrowDown && on_last_line) || (key == Key::ArrowRight && caret == len);
        let n = frozen.read().len();
        let target = if go_prev && i > 0 {
            Some(i - 1)
        } else if go_next && i + 1 < n {
            Some(i + 1)
        } else {
            None
        };
        let Some(t_idx) = target else {
            return;
        };
        e.prevent_default();
        let pos = {
            let f = frozen.read();
            let t = &f[t_idx].text;
            let target_byte = match key {
                // Same column on the prev block's last line.
                Key::ArrowUp => {
                    let ls = t.rfind('\n').map(|x| x + 1).unwrap_or(0);
                    ls + col_to_byte(&t[ls..], column)
                }
                // Same column on the next block's first line.
                Key::ArrowDown => {
                    let le = t.find('\n').unwrap_or(t.len());
                    col_to_byte(&t[..le], column)
                }
                // Left: end of the previous block.
                Key::ArrowLeft => t.len(),
                // Right: start of the next block.
                _ => 0,
            };
            byte_to_utf16(t, target_byte)
        };
        // Skip the blur-deactivate (we're moving focus), then move and place the
        // caret on mount.
        moving.set(true);
        pending_caret.set(Some(pos));
        active.set(Some(t_idx));
    };

    // Inline formatting on the current selection. No JS — just web-sys to
    // read/replace the textarea selection:
    //   * `_` ` ~  (typed)  wrap the selection (press twice for bold/strike); only
    //              with a selection.
    //   Cmd/Ctrl+B / +I      bold / italic (works empty too).
    //   Cmd/Ctrl+K           wrap as a [text](url) link.
    #[cfg(feature = "web")]
    let mut handle_inline_format = move |i: usize, e: &KeyboardEvent| {
        let mods = e.modifiers();
        let cmd = mods.contains(Modifiers::CONTROL) || mods.contains(Modifiers::META);
        let alt = mods.contains(Modifiers::ALT);
        let spec: Option<WrapSpec> = match e.key() {
            Key::Character(c) if cmd && !alt => match c.to_lowercase().as_str() {
                "b" => Some(WrapSpec {
                    open: "**".into(),
                    close: "**".into(),
                    select_url: false,
                    require_selection: false,
                }),
                "i" => Some(WrapSpec {
                    open: "*".into(),
                    close: "*".into(),
                    select_url: false,
                    require_selection: false,
                }),
                "k" => Some(WrapSpec {
                    open: "[".into(),
                    close: "](url)".into(),
                    select_url: true,
                    require_selection: false,
                }),
                _ => None,
            },
            Key::Character(c) if !cmd && !alt && matches!(c.as_str(), "*" | "_" | "`" | "~") => {
                Some(WrapSpec {
                    open: c.clone(),
                    close: c.clone(),
                    select_url: false,
                    require_selection: true,
                })
            }
            _ => None,
        };
        let Some(spec) = spec else {
            return;
        };
        let Some((ta, value, start, end)) = textarea_state(e) else {
            return;
        };
        if spec.require_selection && start == end {
            return;
        }
        e.prevent_default();
        let bs = utf16_to_byte(&value, start);
        let be = utf16_to_byte(&value, end);
        let new = format!(
            "{}{}{}{}{}",
            &value[..bs],
            spec.open,
            &value[bs..be],
            spec.close,
            &value[be..],
        );
        let open16 = spec.open.encode_utf16().count() as u32;
        // New selection / caret (UTF-16):
        let (ns, ne) = if spec.select_url {
            // Select the "url" placeholder so the user types the URL over it.
            let u = end as u32 + open16 + 2;
            (u, u + 3)
        } else if start == end {
            // Empty: caret between the markers.
            (start as u32 + open16, start as u32 + open16)
        } else {
            // Keep the selection on the inner text so a repeat press wraps again.
            (start as u32 + open16, end as u32 + open16)
        };
        // Set the DOM value first so the controlled re-render writes an identical
        // string (no caret reset), then restore the selection.
        ta.set_value(&new);
        let _ = ta.set_selection_range(ns, ne);
        commit_block(i, new);
    };

    // While editing, render the frozen snapshot (stable indices); otherwise the
    // live re-split. Drag-to-reorder is enabled only when not editing a block.
    let view = if active().is_some() {
        frozen()
    } else {
        blocks()
    };
    // Content-derived render keys so reorder / insert reuses the right DOM node (L3).
    let keys = block_keys(&view);
    // Constant key for the active textarea: contains no `#`, so it never collides
    // with a `block_keys` entry, and stays put as the textarea's content mutates.
    let active_key = "active-textarea";
    // Constant key for the textarea's positioning wrapper — like `active_key`, it
    // contains no `#` so it can't collide with a `block_keys` entry.
    let active_wrapper_key = "active-wrapper";
    let arranging = active().is_none();
    // A leading frontmatter block is pinned while arranging: rendered above the
    // drag area with no ⠿ handle and no drop slot, so reordering can never move
    // it or land another block above it. (While editing it's an ordinary
    // clickable row — clicking it opens the raw YAML in the textarea.)
    let pinned = arranging && view.first().is_some_and(|b| b.frontmatter);
    let base = pinned as usize;

    rsx! {
        // riparion-dnd's drop-zone / handle styling, plus a scoped override so idle
        // drop slivers stay slim (riparion-dnd's default 10px reintroduces big gaps).
        document::Stylesheet { href: DEFAULT_STYLE }
        style {
            ".riparion-editor .dnd-dz{{height:3px}}
             .riparion-editor .dnd-dz-tail{{min-height:8px}}
             .riparion-editor .dnd-dz-hover{{height:14px;background:currentColor;opacity:.5}}
             .riparion-editor .prose :where(p,ul,ol,pre,blockquote,table,h1,h2,h3,h4){{margin-top:.35rem;margin-bottom:.35rem}}"
        }
        div { class: "riparion-editor space-y-0.5",
            if arranging {
                // The pinned frontmatter row sits above the drag area: still
                // clickable (activate → raw YAML textarea) and still wrapped by
                // the host chrome, but with no drag handle.
                if pinned {
                    div {
                        key: "{keys[0]}",
                        class: "flex items-center gap-2 {block_class}",
                        {render_row(0, view[0].text.clone(), rsx! {
                            RenderedContent {
                                text: view[0].text.clone(),
                                class: "flex-1".to_string(),
                                render_block,
                                on_activate: move |_| activate.call(0),
                            }
                        })}
                    }
                }
                // Arrange mode: drag a block's ⠿ handle to reorder.
                DragDropArea::<usize> {
                    on_drop: move |evt: DragDropEvent<usize>| on_drop.call(evt),
                    DropList {
                        list_id: "blocks".to_string(),
                        count: view.len(),
                        for (i , blk) in view.iter().enumerate().skip(base) {
                            Draggable::<usize> {
                                key: "{keys[i]}",
                                item_id: i,
                                list_id: "blocks".to_string(),
                                slot: i,
                                class: format!("flex items-center gap-2 {block_class}"),
                                // The host wrapper sits *inside* the draggable
                                // card, so the ⠿ drag handle stays outside any
                                // chrome (e.g. a context-menu trigger).
                                {render_row(i, blk.text.clone(), rsx! {
                                    RenderedContent {
                                        text: blk.text.clone(),
                                        class: "flex-1".to_string(),
                                        render_block,
                                        on_activate: move |_| activate.call(i),
                                    }
                                })}
                            }
                        }
                    }
                }
            } else {
                // Editing mode: the active block is a raw textarea, the rest are
                // clickable rendered blocks (no drag handles while editing).
                for (i , blk) in view.iter().enumerate() {
                    if active() == Some(i) {
                        // Positioning context for the autocomplete popup, which
                        // renders absolutely below the textarea.
                        div {
                            key: "{active_wrapper_key}",
                            style: "position:relative;",
                        textarea {
                            // A constant key (no `#`, so it can't collide with a
                            // `block_keys` entry): keeps the textarea from remounting
                            // — and losing focus — as its content changes on input.
                            key: "{active_key}",
                            class: "{textarea_class}",
                            // Grow to fit: one row per line (+1 of slack), min 2. Keeps
                            // a long block from scrolling inside a short fixed box.
                            rows: ((blk.text.matches('\n').count() + 1).max(2)) as i64,
                            value: blk.text.clone(),
                            // Focus the textarea the instant it mounts so a click on a
                            // block lands the caret without a second click.
                            onmounted: move |e: MountedEvent| {
                                spawn(async move {
                                    let _ = e.set_focus(true).await;
                                    // Place the caret where arrow nav asked (after
                                    // focus, which would otherwise land it at the end).
                                    #[cfg(feature = "web")]
                                    {
                                        let mut pc = pending_caret;
                                        if let Some(pos) = pc() {
                                            pc.set(None);
                                            use dioxus::web::WebEventExt;
                                            use wasm_bindgen::JsCast;
                                            if let Some(el) = e.data().try_as_web_event() {
                                                if let Ok(ta) = el
                                                    .dyn_into::<web_sys::HtmlTextAreaElement>()
                                                {
                                                    let _ = ta.set_selection_range(pos, pos);
                                                }
                                            }
                                        }
                                    }
                                });
                            },
                            oninput: move |e: FormEvent| {
                                commit_block(i, e.value());
                                ac.on_input(&e);
                            },
                            // Leaving the block normalizes spacing so any structural
                            // edit (insert / split) leaves blocks cleanly separated.
                            // A blur caused by a programmatic move to another block is
                            // ignored (see `moving`).
                            onblur: move |_| {
                                if moving() {
                                    moving.set(false);
                                } else {
                                    active.set(None);
                                    body.set(normalize_body(&body(), atomic));
                                }
                            },
                            onkeydown: move |e: KeyboardEvent| {
                                // The autocomplete popup gets first refusal on keys,
                                // so its ↑/↓/Enter/Tab/Esc don't also drive block
                                // split / cross-block nav / deactivate below.
                                match ac.on_keydown(&e) {
                                    KeyOutcome::Accepted { value } => {
                                        commit_block(i, value);
                                        return;
                                    }
                                    KeyOutcome::Consumed => return,
                                    KeyOutcome::Ignored => {}
                                }
                                // Esc renders the block again (works on every target).
                                // The caret-aware behaviors live in the extracted
                                // web-only handlers (K1): double-Enter to split,
                                // arrow nav across blocks, inline-format shortcuts.
                                if e.key() == Key::Escape {
                                    active.set(None);
                                    body.set(normalize_body(&body(), atomic));
                                } else {
                                    #[cfg(feature = "web")]
                                    if e.key() == Key::Enter && e.modifiers().is_empty() {
                                        handle_enter(i, &e);
                                    } else if e.modifiers().is_empty()
                                        && matches!(
                                            e.key(),
                                            Key::ArrowUp
                                                | Key::ArrowDown
                                                | Key::ArrowLeft
                                                | Key::ArrowRight
                                        )
                                    {
                                        handle_arrow_nav(i, &e);
                                    } else {
                                        handle_inline_format(i, &e);
                                    }
                                }
                            },
                        }
                        if ac.open() {
                            CompletionPopup {
                                items: ac.items(),
                                selected: ac.selected(),
                                on_pick: move |idx: usize| {
                                    #[cfg(feature = "web")]
                                    if let Some(value) = ac.accept(idx) {
                                        commit_block(i, value);
                                    }
                                    #[cfg(not(feature = "web"))]
                                    let _ = idx;
                                },
                                menu_class: completion_menu_class.clone(),
                                item_class: completion_item_class.clone(),
                                item_active_class: completion_item_active_class.clone(),
                            }
                        }
                        }
                    } else {
                        {render_row(i, blk.text.clone(), rsx! {
                            RenderedContent {
                                key: "{keys[i]}",
                                text: blk.text.clone(),
                                class: block_class.clone(),
                                render_block,
                                on_activate: move |_| activate.call(i),
                            }
                        })}
                    }
                }
            }
            // Always-present, visible affordance to start a new block. `type=button`
            // so it never submits a surrounding <form>. Self-contained inline style
            // so the crate looks reasonable with zero host CSS.
            button {
                r#type: "button",
                class: "riparion-editor-add",
                style: "display:block;width:100%;text-align:left;cursor:text;opacity:0.45;padding:0.35rem 0.75rem;border:1px dashed currentColor;border-radius:0.5rem;background:transparent;color:inherit;font:inherit;font-size:0.8rem;",
                // mousedown + prevent_default keeps focus on the active textarea, so
                // `active` is still set when we insert below it (a plain onclick would
                // fire after the textarea's blur had already cleared `active`).
                onmousedown: move |e: MouseEvent| {
                    e.prevent_default();
                    add_block.call(());
                },
                "+ New block"
            }
        }
    }
}

/// One inactive, rendered block's content: the rendered markdown, clickable to
/// start editing. A separate component so Dioxus memoizes it on `text`/`class` —
/// while one block is being edited (or another dragged), unchanged rows don't
/// re-run their (possibly expensive) markdown render. Used both as a plain row in
/// edit mode and as the child of a `riparion_dnd::Draggable` card in arrange mode.
#[component]
fn RenderedContent(
    text: String,
    class: String,
    render_block: Callback<String, Element>,
    on_activate: EventHandler<()>,
) -> Element {
    // Whitespace-only blocks render nothing, so give them a thin clickable strip
    // to keep them selectable (e.g. the blank gap before a new paragraph).
    if text.trim().is_empty() {
        return rsx! {
            div {
                class: "{class}",
                style: "min-height:0.5rem",
                onclick: move |_| on_activate.call(()),
            }
        };
    }
    rsx! {
        div {
            class: "{class}",
            onclick: move |_| on_activate.call(()),
            {render_block.call(text.clone())}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_atomic(_: &str) -> bool {
        false
    }

    /// `join ∘ split` must be the identity for any input.
    fn assert_roundtrip(s: &str, is_atomic: impl Fn(&str) -> bool) {
        let blocks = split_blocks(s, is_atomic);
        assert_eq!(join_blocks(&blocks), s, "round-trip failed for {s:?}");
    }

    #[test]
    fn roundtrip_plain_prose() {
        assert_roundtrip("Hello world\n", no_atomic);
        assert_roundtrip("Hello\n\nWorld\n", no_atomic);
        assert_roundtrip("no trailing newline", no_atomic);
        assert_roundtrip("", no_atomic);
    }

    #[test]
    fn roundtrip_leading_and_trailing_blanks() {
        assert_roundtrip("\n\nHello\n", no_atomic);
        assert_roundtrip("Hello\n\n\n", no_atomic);
        assert_roundtrip("\n", no_atomic);
    }

    #[test]
    fn roundtrip_headings_and_lists() {
        let s = "# Title\n\nIntro paragraph.\n\n- one\n- two\n- three\n\nOutro.\n";
        assert_roundtrip(s, no_atomic);
    }

    #[test]
    fn roundtrip_crlf() {
        assert_roundtrip("Hello\r\n\r\nWorld\r\n", no_atomic);
    }

    #[test]
    fn fenced_code_with_blank_line_is_one_block() {
        let s = "Intro\n\n```rust\nlet a = 1;\n\nlet b = 2;\n```\n\nOutro\n";
        let blocks = split_blocks(s, no_atomic);
        assert_eq!(join_blocks(&blocks), s);
        // The fence (including its internal blank line) must be a single block.
        let fence = blocks
            .iter()
            .find(|b| b.text.contains("let a = 1;"))
            .expect("fence block present");
        assert!(
            fence.text.contains("let b = 2;"),
            "fence was split: {fence:?}"
        );
    }

    #[test]
    fn tilde_fence() {
        let s = "~~~\ncode\n\nmore\n~~~\n";
        let blocks = split_blocks(s, no_atomic);
        assert_eq!(join_blocks(&blocks), s);
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn normalize_separates_blocks_and_drops_empties() {
        // Adjacent blocks that would otherwise merge get a single blank line;
        // empty/whitespace blocks are dropped; intra-block soft breaks survive.
        assert_eq!(normalize_body("A\nB\n", no_atomic), "A\nB\n"); // one block, untouched
        assert_eq!(normalize_body("A\n\n\n\nB\n", no_atomic), "A\n\nB\n"); // collapse extra blanks
        assert_eq!(normalize_body("\n\nA\n\n", no_atomic), "A\n"); // drop leading blanks
        assert_eq!(normalize_body("", no_atomic), "");
        // Soft line break inside a block is preserved.
        assert_eq!(
            normalize_body("L1\nL2\n\nP2\n", no_atomic),
            "L1\nL2\n\nP2\n"
        );
    }

    #[test]
    fn indented_4_spaces_is_not_a_fence() {
        // CommonMark: 4-space indentation makes it an indented code block, not a
        // fence — the run must not swallow following lines as code.
        assert_eq!(open_fence("    ```"), None);
        assert_eq!(open_fence("\t```"), None);
        // Up to 3 spaces still opens a fence.
        assert_eq!(open_fence("   ```"), Some(('`', 3)));
        // No fence opens, so the indented ``` and the prose after it stay one
        // ordinary block rather than the prose being swallowed as code.
        let s = "    ```\nstill prose\n";
        let blocks = split_blocks(s, no_atomic);
        assert_eq!(join_blocks(&blocks), s);
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn fence_midparagraph_without_blank_line_is_not_recognized() {
        // A ``` directly after a prose line (no blank between) must not start a
        // fence and swallow the rest of the document.
        let s = "some prose\n```\nnot really code\n";
        let blocks = split_blocks(s, no_atomic);
        assert_eq!(join_blocks(&blocks), s);
        // One block, no open fence left dangling.
        assert_eq!(blocks.len(), 1);
        // A fence DOES open after a blank line, though.
        let s2 = "some prose\n\n```\ncode\n```\n";
        let blocks2 = split_blocks(s2, no_atomic);
        assert_eq!(join_blocks(&blocks2), s2);
        assert!(blocks2.iter().any(|b| b.text.contains("```\ncode\n```")));
    }

    #[test]
    fn atomic_line_is_its_own_block() {
        let is_atomic = |l: &str| l.trim().starts_with("[[");
        let s = "Hello\n\n[[component:chart id=1]]\n\nWorld\n";
        let blocks = split_blocks(s, is_atomic);
        assert_eq!(join_blocks(&blocks), s);
        let embed = blocks.iter().find(|b| b.atomic).expect("atomic block");
        assert_eq!(embed.text, "[[component:chart id=1]]\n");
    }

    #[test]
    fn atomic_line_not_detected_inside_fence() {
        let is_atomic = |l: &str| l.trim().starts_with("[[");
        let s = "```\n[[component:chart]]\n```\n";
        let blocks = split_blocks(s, is_atomic);
        assert_eq!(join_blocks(&blocks), s);
        // The bracket line is inside the fence, so it is NOT its own block.
        assert!(blocks.iter().all(|b| !b.atomic));
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn delete_removes_block_and_normalizes() {
        let bs = split_blocks("A\n\nB\n\nC\n", no_atomic);
        assert_eq!(delete_and_normalize(&bs, 1), "A\n\nC\n");
        assert_eq!(delete_and_normalize(&bs, 0), "B\n\nC\n");
        // Out of range: no-op (body just normalizes).
        assert_eq!(delete_and_normalize(&bs, 9), "A\n\nB\n\nC\n");
    }

    #[test]
    fn delete_last_block_yields_empty() {
        let bs = split_blocks("Only\n", no_atomic);
        assert_eq!(delete_and_normalize(&bs, 0), "");
    }

    #[test]
    fn insert_after_adds_block_with_separator() {
        let bs = split_blocks("A\n\nB\n", no_atomic);
        assert_eq!(
            insert_after_and_normalize(&bs, 0, "X", no_atomic),
            "A\n\nX\n\nB\n"
        );
        // After the last block.
        assert_eq!(
            insert_after_and_normalize(&bs, 1, "Z", no_atomic),
            "A\n\nB\n\nZ\n"
        );
        // Whitespace-only paste is dropped by normalization.
        assert_eq!(
            insert_after_and_normalize(&bs, 0, "   ", no_atomic),
            "A\n\nB\n"
        );
    }

    #[test]
    fn insert_after_multiparagraph_paste_splits() {
        let bs = split_blocks("A\n", no_atomic);
        assert_eq!(
            insert_after_and_normalize(&bs, 0, "P1\n\nP2", no_atomic),
            "A\n\nP1\n\nP2\n"
        );
    }

    #[test]
    fn frontmatter_kept_as_one_block() {
        let s = "---\ntitle: Hi\ntags: [a, b]\n---\n\n# Body\n";
        let blocks = split_blocks(s, no_atomic);
        assert_eq!(join_blocks(&blocks), s);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].frontmatter);
        // The blank separator attaches to the frontmatter block (no spacer).
        assert_eq!(blocks[0].text, "---\ntitle: Hi\ntags: [a, b]\n---\n\n");
        assert!(!blocks[1].frontmatter);
        assert_eq!(blocks[1].text, "# Body\n");
    }

    #[test]
    fn frontmatter_roundtrips() {
        // Hostile interior: an atomic-looking line and a fence opener must not
        // split the frontmatter or leak fence state into the body.
        let s =
            "---\ntitle: Hi\n[[embed]]\n```\nnote: fence chars\n---\n\nBody\n\n```\ncode\n```\n";
        let embeds = |l: &str| l.trim_start().starts_with("[[");
        assert_roundtrip(s, embeds);
        let blocks = split_blocks(s, embeds);
        assert!(blocks[0].frontmatter);
        assert!(blocks[0].text.ends_with("---\n\n"));
        assert!(blocks[0].text.contains("[[embed]]"));
        // The body fence is still one block.
        let fence = blocks
            .iter()
            .find(|b| b.text.contains("code"))
            .expect("fence block present");
        assert!(fence.text.contains("```\ncode\n```"));
    }

    #[test]
    fn frontmatter_blank_line_inside_does_not_split() {
        let s = "---\ntitle: Hi\n\ndate: 2026-06-04\n---\nBody\n";
        let blocks = split_blocks(s, no_atomic);
        assert_eq!(join_blocks(&blocks), s);
        assert!(blocks[0].frontmatter);
        assert!(blocks[0].text.contains("date:"));
    }

    #[test]
    fn frontmatter_only_at_offset_zero() {
        let s = "Intro\n\n---\na: 1\n---\n";
        let blocks = split_blocks(s, no_atomic);
        assert_eq!(join_blocks(&blocks), s);
        assert!(blocks.iter().all(|b| !b.frontmatter));
    }

    #[test]
    fn frontmatter_leading_blank_is_not_frontmatter() {
        let s = "\n---\na: 1\n---\n";
        let blocks = split_blocks(s, no_atomic);
        assert_eq!(join_blocks(&blocks), s);
        assert!(blocks.iter().all(|b| !b.frontmatter));
    }

    #[test]
    fn triple_dash_thematic_break_unchanged() {
        // A `---` after prose is a thematic break / setext underline, not
        // frontmatter — splitting behaves exactly as before.
        let s = "Title\n---\n\nBody\n";
        let blocks = split_blocks(s, no_atomic);
        assert_eq!(join_blocks(&blocks), s);
        assert!(blocks.iter().all(|b| !b.frontmatter));
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn unclosed_frontmatter_is_not_frontmatter() {
        // A lone leading `---` must not swallow the document.
        let s = "---\ntitle: Hi\n\nBody\n";
        assert_eq!(frontmatter_len(s), None);
        let blocks = split_blocks(s, no_atomic);
        assert_eq!(join_blocks(&blocks), s);
        assert!(blocks.iter().all(|b| !b.frontmatter));
        assert_eq!(blocks.len(), 2); // splits on the blank line as usual
    }

    #[test]
    fn empty_frontmatter() {
        let s = "---\n---\nBody\n";
        let blocks = split_blocks(s, no_atomic);
        assert_eq!(join_blocks(&blocks), s);
        assert!(blocks[0].frontmatter);
        assert_eq!(blocks[0].text, "---\n---\n");
    }

    #[test]
    fn frontmatter_only_document() {
        let s = "---\ntitle: Hi\n---\n";
        let blocks = split_blocks(s, no_atomic);
        assert_eq!(join_blocks(&blocks), s);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].frontmatter);
        // Closing line without a trailing newline still closes.
        let s2 = "---\ntitle: Hi\n---";
        assert_eq!(frontmatter_len(s2), Some(s2.len()));
        assert_roundtrip(s2, no_atomic);
    }

    #[test]
    fn frontmatter_dot_close() {
        // YAML's document-end marker `...` also closes the run.
        let s = "---\ntitle: Hi\n...\n\nBody\n";
        let blocks = split_blocks(s, no_atomic);
        assert_eq!(join_blocks(&blocks), s);
        assert!(blocks[0].frontmatter);
        assert!(blocks[0].text.ends_with("...\n\n"));
    }

    #[test]
    fn frontmatter_crlf() {
        let s = "---\r\ntitle: Hi\r\n---\r\n\r\nBody\r\n";
        let blocks = split_blocks(s, no_atomic);
        assert_eq!(join_blocks(&blocks), s);
        assert!(blocks[0].frontmatter);
    }

    #[test]
    fn frontmatter_no_blank_before_body() {
        let s = "---\ntitle: Hi\n---\nBody\n";
        let blocks = split_blocks(s, no_atomic);
        assert_eq!(join_blocks(&blocks), s);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].frontmatter);
        assert_eq!(blocks[1].text, "Body\n");
    }

    #[test]
    fn normalize_preserves_frontmatter() {
        // The YAML (including its interior blank line) survives normalization
        // verbatim; the body separator is canonicalized.
        let s = "---\ntitle: Hi\n\ndate: x\n---\n\n\n\nBody\n";
        assert_eq!(
            normalize_body(s, no_atomic),
            "---\ntitle: Hi\n\ndate: x\n---\n\nBody\n"
        );
    }

    #[test]
    fn normalize_frontmatter_only() {
        assert_eq!(
            normalize_body("---\ntitle: Hi\n---", no_atomic),
            "---\ntitle: Hi\n---\n"
        );
    }
}
