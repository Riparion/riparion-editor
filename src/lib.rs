//! A reusable, pure-Dioxus **block-swap live-preview** markdown editor.
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
use dx_dnd::{DragDropArea, DragDropEvent, Draggable, DropList, DEFAULT_STYLE};

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
            });
        }
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

        if let Some(open) = open_fence(content) {
            if closed {
                flush(&mut cur, &mut blocks);
                closed = false;
            }
            cur.push_str(line);
            fence = Some(open);
            continue;
        }

        if is_atomic(content) {
            flush(&mut cur, &mut blocks);
            blocks.push(Block {
                text: line.to_string(),
                atomic: true,
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
fn open_fence(content: &str) -> Option<(char, usize)> {
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

/// Join blocks with canonical single-blank-line separators, dropping empty /
/// whitespace-only blocks. Only the separators *between* blocks are touched —
/// each block's own content (leading indentation, soft line breaks) is preserved.
/// Used after a *structural* edit (insert, reorder) so adjacent blocks can't merge
/// or pile up stray blank lines.
pub fn rejoin_normalized(blocks: &[Block]) -> String {
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

/// Read `(value, caret_byte_offset)` from the textarea that fired key event `e`,
/// translating the browser's UTF-16 `selectionStart` into a Rust byte index so
/// slicing the value is correct for multi-byte text. `None` off the web target.
#[cfg(feature = "web")]
fn textarea_caret(e: &KeyboardEvent) -> Option<(String, usize)> {
    use dioxus::web::WebEventExt;
    use wasm_bindgen::JsCast;
    let we: web_sys::KeyboardEvent = e.data().try_as_web_event()?;
    let ta = we
        .target()?
        .dyn_into::<web_sys::HtmlTextAreaElement>()
        .ok()?;
    let value = ta.value();
    let caret16 = ta.selection_start().ok().flatten()? as usize;
    let mut units = 0usize;
    for (byte, ch) in value.char_indices() {
        if units >= caret16 {
            return Some((value, byte));
        }
        units += ch.len_utf16();
    }
    let len = value.len();
    Some((value, len))
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
) -> Element {
    let mut body = body;
    // Index of the block currently being edited as raw markdown, if any.
    let mut active = use_signal(|| Option::<usize>::None);
    // Snapshot taken when a block is activated. While active we render from this
    // snapshot and never re-split, so the active index and the textarea's caret
    // stay stable no matter what the user types (including blank lines).
    let mut frozen = use_signal(Vec::<Block>::new);
    // Set when we *programmatically* move focus to another block (double-Enter).
    // Removing the old textarea fires a `blur`; without this guard that blur would
    // immediately deactivate the editor and normalize away the new empty block.
    let mut moving = use_signal(|| false);

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
        let mut snap = split_blocks(&body(), atomic);
        let at = match active() {
            Some(i) => (i + 1).min(snap.len()),
            None => snap.len(),
        };
        if at > 0 {
            let prev = &mut snap[at - 1].text;
            if !prev.is_empty() && !prev.ends_with("\n\n") {
                if prev.ends_with('\n') {
                    prev.push('\n');
                } else {
                    prev.push_str("\n\n");
                }
            }
        }
        snap.insert(
            at,
            Block {
                text: String::new(),
                atomic: false,
            },
        );
        active.set(Some(at));
        body.set(join_blocks(&snap));
        frozen.set(snap);
    });

    // Drag-and-drop block reordering, via the dx-dnd primitives. The drop event
    // carries source/target slots; we move the block and rejoin with canonical
    // separators (reorder is a structural edit, so spacing is normalized).
    let on_drop = use_callback(move |evt: DragDropEvent<usize>| {
        let mut bs = split_blocks(&body(), atomic);
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

    // While editing, render the frozen snapshot (stable indices); otherwise the
    // live re-split. Drag-to-reorder is enabled only when not editing a block.
    let view = if active().is_some() {
        frozen()
    } else {
        blocks()
    };
    let arranging = active().is_none();

    rsx! {
        // dx-dnd's drop-zone / handle styling, plus a scoped override so idle
        // drop slivers stay slim (dx-dnd's default 10px reintroduces big gaps).
        document::Stylesheet { href: DEFAULT_STYLE }
        style {
            ".riparion-editor .dnd-dz{{height:3px}}
             .riparion-editor .dnd-dz-tail{{min-height:8px}}
             .riparion-editor .dnd-dz-hover{{height:14px;background:currentColor;opacity:.5}}
             .riparion-editor .prose :where(p,ul,ol,pre,blockquote,table,h1,h2,h3,h4){{margin-top:.35rem;margin-bottom:.35rem}}"
        }
        div { class: "riparion-editor space-y-0.5",
            if arranging {
                // Arrange mode: drag a block's ⠿ handle to reorder.
                DragDropArea::<usize> {
                    on_drop: move |evt: DragDropEvent<usize>| on_drop.call(evt),
                    DropList {
                        list_id: "blocks".to_string(),
                        count: view.len(),
                        for (i , blk) in view.iter().enumerate() {
                            Draggable::<usize> {
                                key: "{i}",
                                item_id: i,
                                list_id: "blocks".to_string(),
                                slot: i,
                                class: format!("flex items-start gap-2 {block_class}"),
                                RenderedContent {
                                    text: blk.text.clone(),
                                    class: "flex-1".to_string(),
                                    render_block,
                                    on_activate: move |_| activate.call(i),
                                }
                            }
                        }
                    }
                }
            } else {
                // Editing mode: the active block is a raw textarea, the rest are
                // clickable rendered blocks (no drag handles while editing).
                for (i , blk) in view.iter().enumerate() {
                    if active() == Some(i) {
                        textarea {
                            key: "{i}",
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
                                });
                            },
                            oninput: move |e: FormEvent| {
                                {
                                    let mut f = frozen.write();
                                    if let Some(b) = f.get_mut(i) {
                                        b.text = e.value();
                                    }
                                }
                                body.set(join_blocks(&frozen.read()));
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
                                // Double-Enter (Notion-style): a plain Enter pressed on
                                // a blank line ends this block and opens a fresh one
                                // below, moving the caret into it — fingers stay on the
                                // keyboard. A single Enter still inserts a newline, so
                                // lists/code type normally; Shift/Ctrl/Cmd+Enter alone.
                                if e.key() == Key::Escape {
                                    active.set(None);
                                    body.set(normalize_body(&body(), atomic));
                                } else if e.key() == Key::Enter && e.modifiers().is_empty()
                                {
                                    // Needs the live caret position; web-only (the
                                    // server never runs this interactively).
                                    #[cfg(feature = "web")]
                                    if let Some((value, caret)) = textarea_caret(&e) {
                                        let line_start = value[..caret]
                                            .rfind('\n')
                                            .map(|n| n + 1)
                                            .unwrap_or(0);
                                        let line_end = value[caret..]
                                            .find('\n')
                                            .map(|n| caret + n)
                                            .unwrap_or(value.len());
                                        if value[line_start..line_end].trim().is_empty() {
                                            e.prevent_default();
                                            // Head keeps everything up to the blank line
                                            // and ends in a blank-line separator; the
                                            // rest becomes a new block the caret enters.
                                            let mut head = value[..line_start].to_string();
                                            if !head.is_empty() && !head.ends_with("\n\n") {
                                                if head.ends_with('\n') {
                                                    head.push('\n');
                                                } else {
                                                    head.push_str("\n\n");
                                                }
                                            }
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
                                                        Block { text: tail, atomic: false },
                                                    );
                                                }
                                            }
                                            body.set(join_blocks(&frozen.read()));
                                            // The blur from removing this textarea must
                                            // not deactivate us — we're moving focus.
                                            moving.set(true);
                                            active.set(Some(i + 1));
                                        }
                                    }
                                }
                            },
                        }
                    } else {
                        RenderedContent {
                            key: "{i}",
                            text: blk.text.clone(),
                            class: block_class.clone(),
                            render_block,
                            on_activate: move |_| activate.call(i),
                        }
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
/// edit mode and as the child of a `dx_dnd::Draggable` card in arrange mode.
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
}
