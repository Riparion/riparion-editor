//! A runnable [`BlockEditor`] demo with a real Markdown renderer.
//!
//! It plugs [`pulldown-cmark`] into the crate's `render_block` callback, so each
//! inactive block renders as HTML and the block you click swaps to a raw
//! `<textarea>`. Lines that look like `[[embed]]` are marked atomic via
//! `is_atomic` and render as a standalone card.
//!
//! The document's leading YAML frontmatter renders as an Obsidian-style
//! **Properties card**: click a value to edit that field inline, add/remove
//! fields, or hit the `{}` YAML button to edit the raw block in the normal
//! textarea. `File ▸ Add properties` prepends frontmatter when there is none.
//!
//! A menubar above the editor (dx catalog `menubar` + `alert_dialog`, installed
//! with `dx components add --module-path examples/markdown/components …`) adds
//! File ▸ New (confirm, then clear), File ▸ Open / Save / Save As (OS-native
//! dialogs via the File System Access API where available — Save writes back
//! to the opened file in place; elsewhere Open falls back to `<input
//! type=file>` and Save to a `.md` download), and an About dialog.
//! Right-clicking a rendered block (dx catalog `context_menu`) offers
//! Cut / Copy / Paste / Delete via the system clipboard.
//!
//! Run it in a browser with the Dioxus CLI (the `web` feature pulls in the
//! caret/selection/drag mechanics):
//!
//! ```sh
//! dx serve --example markdown --features web
//! ```

mod components;

use components::alert_dialog::{
    AlertDialog, AlertDialogAction, AlertDialogActions, AlertDialogCancel, AlertDialogDescription,
    AlertDialogTitle,
};
use components::context_menu::{
    ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuTrigger,
};
use components::menubar::{Menubar, MenubarContent, MenubarItem, MenubarMenu, MenubarTrigger};
use components::switch::Switch;
use dioxus::prelude::*;
use pulldown_cmark::{html, Options, Parser};
use riparion_editor::{frontmatter_len, BlockChrome, BlockEditor, CompletionItem};

/// The demo's smart-tag catalog. A real app injects its own; here a couple of
/// fixed entries show the `[[/` autocomplete working.
fn completions(query: String) -> Vec<CompletionItem> {
    let q = query.to_lowercase();
    [
        (
            "embed",
            "Embed",
            "A standalone atomic line.",
            "[[embed: …]]",
        ),
        ("note", "Note", "A callout block.", "[[/note text=\"…\"]]"),
        (
            "chart",
            "Chart",
            "An inline bar chart.",
            "[[/chart data=\"3,7,2,9\"]]",
        ),
        (
            "warning",
            "Warning",
            "A warning callout.",
            "[[/warning text=\"…\"]]",
        ),
        (
            "tip",
            "Tip",
            "A helpful tip callout.",
            "[[/tip text=\"…\"]]",
        ),
        (
            "table",
            "Table",
            "A data table.",
            "[[/table cols=\"a,b,c\"]]",
        ),
        (
            "image",
            "Image",
            "An inline image.",
            "[[/image src=\"…\" alt=\"…\"]]",
        ),
        (
            "video",
            "Video",
            "An embedded video.",
            "[[/video src=\"…\"]]",
        ),
        (
            "code",
            "Code",
            "A syntax-highlighted snippet.",
            "[[/code lang=\"rust\"]]",
        ),
        ("math", "Math", "A LaTeX math block.", "[[/math tex=\"…\"]]"),
        (
            "quote",
            "Quote",
            "A block quotation.",
            "[[/quote cite=\"…\"]]",
        ),
        (
            "toc",
            "Contents",
            "A table of contents.",
            "[[/toc depth=\"2\"]]",
        ),
        ("divider", "Divider", "A horizontal rule.", "[[/divider]]"),
    ]
    .into_iter()
    .filter(|(name, label, _, _)| {
        q.is_empty() || name.contains(&q) || label.to_lowercase().contains(&q)
    })
    .map(|(_, label, detail, insert)| CompletionItem {
        label: label.to_string(),
        detail: detail.to_string(),
        insert: insert.to_string(),
    })
    .collect()
}

/// Seed document, exercising YAML frontmatter (rendered as the Properties
/// card), headings, lists, a fenced code block, and an atomic `[[embed]]` line.
/// The `tags` sequence demonstrates the card's graceful degradation: non-scalar
/// values show read-only and are edited through the `{}` YAML escape hatch.
const SEED: &str = r#"---
title: riparion-editor
tags: [editor, dioxus, markdown]
date: 2026-06-04
draft: false
---

# riparion-editor

A **block-swap** live-preview editor. Click any block to edit its raw Markdown;
click away (or press Esc) to render it again.

## Try it

- Click a value in the Properties card above to edit it
- Click this list to edit it
- Press *double-Enter* to start a new block
- Drag the handle on the left to reorder blocks
- Select text and type `*`, `` ` `` or `~` to wrap it

```rust
fn main() {
    println!("fenced code stays one block");
}
```

[[embed: this whole line is atomic — edit just it]]

That's the whole idea — bring your own renderer, keep your own source.
"#;

/// A tiny global stylesheet so the demo is legible without a CSS framework.
const STYLES: &str = r#"
/* Theme: the menubar's Switch sets `data-theme` on <html>, which
   dx-components-theme.css turns into the `--light`/`--dark` space toggles
   (one is empty, the other is the guaranteed-invalid `initial`, so
   `var(--light, X) var(--dark, Y)` collapses to X or Y). The demo's own
   colors below use the same pattern so the whole page follows the switch,
   not just the dx widgets. */
body { margin: 0;
    background: var(--light, #f6f7f9) var(--dark, #101113);
    color: var(--light, #1a1a1a) var(--dark, #e4e6ea);
    font: 16px/1.6 system-ui, -apple-system, sans-serif; }
.page { max-width: 44rem; margin: 0 auto; padding: 2rem 1rem 6rem; }
.toolbar { margin-bottom: 1.25rem; }
.theme-toggle { display: flex; align-items: center; gap: 0.5rem;
    margin-left: auto; padding-right: 0.5rem; font-size: 14px; }
/* In arrange mode BlockEditor tags each draggable row with the utility
   classes `flex items-center gap-2` (and `flex-1` on the content). This demo
   has no CSS framework, so define them here — `align-items: center` is what
   vertically centers the drag handle against its content block. */
.flex { display: flex; }
.items-center { align-items: center; }
.gap-2 { gap: 0.5rem; }
.flex-1 { flex: 1 1 0%; min-width: 0; }
.prose { padding: 0.15rem 0.4rem; border-radius: 0.35rem; }
.prose:hover { background: var(--light, #eceef1) var(--dark, #1c2027); }
.prose pre { background: #1e2430; color: #e7eaf0; padding: 0.9rem 1rem;
    border-radius: 0.5rem; overflow-x: auto; }
.prose code { font-family: ui-monospace, SFMono-Regular, monospace; font-size: 0.9em; }
.prose pre code { background: none; padding: 0; }
.embed { border: 1px dashed var(--light, #9aa3af) var(--dark, #4b5563);
    border-radius: 0.5rem; padding: 0.75rem 1rem;
    background: var(--light, #fff) var(--dark, #16181c);
    color: var(--light, #4b5563) var(--dark, #9aa3af); font-style: italic; }
.editor-textarea { width: 100%; box-sizing: border-box; padding: 0.4rem 0.5rem;
    border: 1px solid var(--light, #c7ccd3) var(--dark, #3a3f47);
    border-radius: 0.4rem; background: var(--light, #fff) var(--dark, #16181c);
    color: inherit;
    font-family: ui-monospace, SFMono-Regular, monospace; font-size: 0.95rem; }
.ac-menu { min-width: 18rem; background: var(--light, #fff) var(--dark, #16181c);
    border: 1px solid var(--light, #c7ccd3) var(--dark, #3a3f47);
    border-radius: 0.5rem; box-shadow: 0 8px 24px rgba(0,0,0,0.12); padding: 0.25rem; }
.ac-item { padding: 0.35rem 0.6rem; border-radius: 0.35rem; }
.ac-item:hover { background: var(--light, #eceef1) var(--dark, #1c2027); }
.ac-item-active { background: var(--light, #e3effe) var(--dark, #1e3a5f); }
/* Frontmatter "Properties" card. */
.fm-card { border: 1px solid var(--light, #c7ccd3) var(--dark, #3a3f47);
    border-radius: 0.5rem; padding: 0.5rem 0.75rem;
    background: var(--light, #fff) var(--dark, #16181c); font-size: 0.9rem; }
.fm-header { display: flex; align-items: center; justify-content: space-between;
    margin-bottom: 0.25rem; }
.fm-title { font-weight: 600; font-size: 0.75rem; text-transform: uppercase;
    letter-spacing: 0.06em; color: var(--light, #6b7280) var(--dark, #9aa3af); }
.fm-yaml-btn, .fm-add, .fm-x { border: none; background: none; cursor: pointer;
    color: var(--light, #6b7280) var(--dark, #9aa3af);
    border-radius: 0.3rem; font-size: 0.8rem; padding: 0.1rem 0.4rem; }
.fm-yaml-btn { font-family: ui-monospace, SFMono-Regular, monospace; }
.fm-yaml-btn:hover, .fm-add:hover, .fm-x:hover {
    background: var(--light, #eceef1) var(--dark, #1c2027);
    color: inherit; }
.fm-row { display: grid; grid-template-columns: 9rem 1fr auto; gap: 0.5rem;
    align-items: baseline; padding: 0.15rem 0.25rem; border-radius: 0.3rem; }
.fm-row:hover { background: var(--light, #f3f4f6) var(--dark, #1c2027); }
.fm-key { color: var(--light, #6b7280) var(--dark, #9aa3af);
    font-family: ui-monospace, SFMono-Regular, monospace; font-size: 0.85em;
    overflow-wrap: anywhere; }
.fm-value { cursor: text; min-height: 1.2em; overflow-wrap: anywhere; }
.fm-locked { cursor: default; opacity: 0.75;
    font-family: ui-monospace, SFMono-Regular, monospace; font-size: 0.9em; }
.fm-raw { grid-column: 1 / 4; opacity: 0.6; white-space: pre-wrap;
    font-family: ui-monospace, SFMono-Regular, monospace; font-size: 0.85em; }
.fm-empty { opacity: 0.5; font-style: italic; }
.fm-add { justify-self: start; margin-top: 0.15rem; }
.fm-input { box-sizing: border-box; width: 100%; padding: 0.1rem 0.3rem;
    border: 1px solid var(--light, #c7ccd3) var(--dark, #3a3f47);
    border-radius: 0.3rem; background: var(--light, #fff) var(--dark, #101113);
    color: inherit; font: inherit; }
"#;

fn main() {
    dioxus::launch(app);
}

fn app() -> Element {
    let mut body = use_signal(|| SEED.to_string());
    // `File ▸ New` is destructive, so it routes through a confirmation dialog
    // instead of clearing the editor outright.
    let mut confirm_new = use_signal(|| false);
    let mut about_open = use_signal(|| false);

    // Name of the file we opened / last saved as. Used as the Save As
    // suggestion; the writable handle itself lives JS-side (a
    // `FileSystemFileHandle` can't cross the wasm boundary).
    let mut file_name = use_signal(|| "document.md".to_string());

    // Light/dark theme, toggled by the switch at the right end of the menubar.
    // dx-components-theme.css keys its color tokens off `<html data-theme=…>`,
    // and the demo's own styles follow the same toggles (see STYLES).
    let mut dark = use_signal(|| false);
    use_effect(move || {
        let theme = if dark() { "dark" } else { "light" };
        document::eval(&format!(
            "document.documentElement.dataset.theme = {theme:?};"
        ));
    });

    // One dispatcher for every menu item; the item's `value` says what to do.
    let on_menu = move |value: String| match value.as_str() {
        "new" => confirm_new.set(true),
        "open" => {
            spawn(async move {
                if let Some((name, text)) = open_markdown().await {
                    body.set(text);
                    file_name.set(name);
                }
            });
        }
        "save" | "saveas" => {
            // `Save` writes back through the open file handle when there is
            // one; `Save As` always raises the picker. Both fall back to a
            // download where the File System Access API is unavailable.
            let force_picker = value == "saveas";
            let content = body.peek().clone();
            let suggested = file_name.peek().clone();
            spawn(async move {
                if let Some(name) = save_markdown(content, suggested, force_picker).await {
                    file_name.set(name);
                }
            });
        }
        "props" => {
            // Prepend a starter frontmatter run — but only when the document
            // doesn't already open with one.
            let doc = body.peek().clone();
            if frontmatter_len(&doc).is_none() {
                body.set(format!("---\ntitle: Untitled\n---\n\n{doc}"));
            }
        }
        "about" => about_open.set(true),
        _ => {}
    };

    // Route the document's leading frontmatter block to the Properties card;
    // everything else renders through pulldown-cmark. The block must be a whole
    // `---…---` run (plus its absorbed blank separator) *and* be the document
    // prefix — a lookalike block later in the document stays ordinary Markdown.
    let render_block_or_card = use_callback(move |src: String| {
        let is_doc_frontmatter = frontmatter_len(&src)
            .is_some_and(|len| src[len..].trim().is_empty())
            && body.peek().starts_with(src.as_str());
        if is_doc_frontmatter {
            rsx! {
                FrontmatterCard { text: src, body }
            }
        } else {
            render_block(src)
        }
    });

    rsx! {
        document::Style { {STYLES} }
        // Design tokens (colors, radii, focus rings) shared by the dx catalog
        // widgets under `components/`.
        document::Link { rel: "stylesheet", href: asset!("/assets/dx-components-theme.css") }
        // Pin every catalog widget's `#[css_module]` stylesheet from this
        // permanently-mounted root: the macro's own OnceLock <link> injection
        // drops on remounts, leaving the widgets unstyled. The browser
        // de-dupes by href, so the in-widget emits stay harmless.
        document::Stylesheet { href: components::menubar::MENUBAR_CSS }
        document::Stylesheet { href: components::switch::SWITCH_CSS }
        document::Stylesheet { href: components::alert_dialog::ALERT_DIALOG_CSS }
        document::Stylesheet { href: components::context_menu::CONTEXT_MENU_CSS }
        div { class: "page",
            div { class: "toolbar",
                Menubar {
                    MenubarMenu { index: 0usize,
                        MenubarTrigger { "File" }
                        MenubarContent {
                            MenubarItem { index: 0usize, value: "new", on_select: on_menu, "New" }
                            MenubarItem { index: 1usize, value: "open", on_select: on_menu, "Open…" }
                            MenubarItem { index: 2usize, value: "save", on_select: on_menu, "Save" }
                            MenubarItem {
                                index: 3usize,
                                value: "saveas",
                                on_select: on_menu,
                                "Save As…"
                            }
                            MenubarItem {
                                index: 4usize,
                                value: "props",
                                on_select: on_menu,
                                "Add properties"
                            }
                        }
                    }
                    MenubarMenu { index: 1usize,
                        MenubarTrigger { "About" }
                        MenubarContent {
                            MenubarItem {
                                index: 0usize,
                                value: "about",
                                on_select: on_menu,
                                "About riparion-editor"
                            }
                        }
                    }
                    // `margin-left: auto` floats the toggle to the menubar's
                    // right edge (.dx-menubar is a flex row).
                    div { class: "theme-toggle",
                        span { if dark() { "🌙" } else { "☀️" } }
                        Switch {
                            checked: dark(),
                            on_checked_change: move |v| dark.set(v),
                        }
                    }
                }
            }
            BlockEditor {
                body,
                render_block: render_block_or_card,
                // Any line starting with `[[` is its own atomic block.
                is_atomic: Some(Callback::new(|line: String| {
                    line.trim_start().starts_with("[[")
                })),
                block_class: "prose".to_string(),
                textarea_class: "editor-textarea".to_string(),
                // Type `[[/` in any block to open the autocomplete popup.
                complete: Callback::new(completions),
                completion_menu_class: "ac-menu".to_string(),
                completion_item_class: "ac-item".to_string(),
                completion_item_active_class: "ac-item-active".to_string(),
                // Right-click any rendered block for Cut / Copy / Paste / Delete
                // (dx catalog `context_menu`). Cut/Copy go through the system
                // clipboard; Paste inserts the clipboard below the block.
                wrap_block: Callback::new(block_context_menu),
            }
        }

        // `File ▸ New` — confirm before discarding the document.
        AlertDialog {
            open: confirm_new(),
            on_open_change: move |v| confirm_new.set(v),
            AlertDialogTitle { "Start a new document?" }
            AlertDialogDescription { "This clears the editor. Unsaved changes will be lost." }
            AlertDialogActions {
                AlertDialogCancel { "Cancel" }
                AlertDialogAction {
                    on_click: move |_| {
                        body.set(String::new());
                        // Detach the opened file: a Save on the fresh document
                        // must not overwrite whatever was open before.
                        file_name.set("document.md".to_string());
                        document::eval("window.__riparionFileHandle = null;");
                    },
                    "Clear editor"
                }
            }
        }

        // `About` — a plain informational dialog.
        AlertDialog {
            open: about_open(),
            on_open_change: move |v| about_open.set(v),
            AlertDialogTitle { "About riparion-editor" }
            AlertDialogDescription {
                "A block-swap live-preview Markdown editor for Dioxus: inactive "
                "blocks render as HTML, the block you click swaps to a raw "
                "textarea. Bring your own renderer; drag to reorder."
            }
            AlertDialogActions {
                AlertDialogCancel { "OK" }
            }
        }
    }
}

/// Wrap one rendered block in a right-click context menu (dx catalog
/// `context_menu`) offering Cut / Copy / Paste / Delete. The editor hands us the
/// block's index, its Markdown source, the rendered child to wrap, and the
/// structural ops to call — so the menu is pure example-side chrome.
fn block_context_menu(chrome: BlockChrome) -> Element {
    let BlockChrome {
        index,
        text,
        children,
        delete,
        insert_after,
    } = chrome;
    let cut_text = text.clone();
    rsx! {
        ContextMenu {
            ContextMenuTrigger {
                // In arrange mode the trigger is a flex child next to the drag
                // handle; let it fill the row (no-op in editing mode).
                style: "flex: 1 1 0%; min-width: 0;",
                {children}
            }
            ContextMenuContent {
                ContextMenuItem {
                    index: 0usize,
                    value: "cut".to_string(),
                    on_select: move |_| {
                        clipboard_write(cut_text.clone());
                        delete.call(index);
                    },
                    "Cut"
                }
                ContextMenuItem {
                    index: 1usize,
                    value: "copy".to_string(),
                    on_select: move |_| clipboard_write(text.clone()),
                    "Copy"
                }
                ContextMenuItem {
                    index: 2usize,
                    value: "paste".to_string(),
                    on_select: move |_| {
                        spawn(async move {
                            let pasted = clipboard_read().await;
                            if !pasted.trim().is_empty() {
                                insert_after.call((index, pasted));
                            }
                        });
                    },
                    "Paste"
                }
                ContextMenuItem {
                    index: 3usize,
                    value: "delete".to_string(),
                    on_select: move |_| delete.call(index),
                    "Delete"
                }
            }
        }
    }
}

/// Write `text` to the system clipboard. Best-effort: clipboard access can be
/// denied (insecure context, revoked permission), in which case the failure is
/// logged to the console and the action is a no-op.
fn clipboard_write(text: String) {
    let eval = document::eval(
        r#"
        const text = await dioxus.recv();
        try { await navigator.clipboard.writeText(text); }
        catch (e) { console.warn("clipboard write failed:", e); }
        "#,
    );
    let _ = eval.send(text);
}

/// Read the system clipboard, returning `""` when denied or empty.
async fn clipboard_read() -> String {
    let mut eval = document::eval(
        r#"
        try { dioxus.send(await navigator.clipboard.readText() ?? ""); }
        catch (e) { console.warn("clipboard read failed:", e); dioxus.send(""); }
        "#,
    );
    eval.recv::<String>().await.unwrap_or_default()
}

/// `File ▸ Open` — the OS-native open dialog via the File System Access API
/// (Chromium). The returned `FileSystemFileHandle` is stashed JS-side in
/// `window.__riparionFileHandle` so a later Save can write back in place.
/// Where the API is missing (Firefox, Safari) a hidden `<input type=file>`
/// supplies the same dialog without the writable handle. Returns
/// `(file_name, contents)`, or `None` when the user cancels.
async fn open_markdown() -> Option<(String, String)> {
    let mut eval = document::eval(
        r#"
        const MD = [{ description: "Markdown",
                      accept: { "text/markdown": [".md", ".markdown", ".txt"] } }];
        try {
            if (window.showOpenFilePicker) {
                const [handle] = await window.showOpenFilePicker({ types: MD });
                window.__riparionFileHandle = handle;
                const file = await handle.getFile();
                dioxus.send([file.name, await file.text()]);
            } else {
                const input = document.createElement("input");
                input.type = "file";
                input.accept = ".md,.markdown,.txt,text/markdown";
                const picked = await new Promise((resolve) => {
                    input.onchange = () => resolve(input.files?.[0] ?? null);
                    input.oncancel = () => resolve(null);
                    input.click();
                });
                window.__riparionFileHandle = null;
                dioxus.send(picked ? [picked.name, await picked.text()] : null);
            }
        } catch (e) {
            // AbortError is the user dismissing the picker — not a failure.
            if (e?.name !== "AbortError") console.warn("open failed:", e);
            dioxus.send(null);
        }
        "#,
    );
    eval.recv::<Option<(String, String)>>().await.ok().flatten()
}

/// `File ▸ Save` / `File ▸ Save As` — write through the File System Access API
/// when available: Save reuses the handle stashed by Open / a previous Save As
/// (a true in-place save), while `force_picker` (Save As) always raises the
/// OS-native save dialog. Browsers without the API get the old behavior — a
/// Blob + temporary object-URL anchor click, i.e. a download. Returns the
/// saved file's name, or `None` when the user cancels the picker.
async fn save_markdown(
    content: String,
    suggested_name: String,
    force_picker: bool,
) -> Option<String> {
    let mut eval = document::eval(
        r#"
        const [text, suggested, forcePicker] = await dioxus.recv();
        try {
            let handle = forcePicker ? null : window.__riparionFileHandle;
            if (!handle && window.showSaveFilePicker) {
                handle = await window.showSaveFilePicker({
                    suggestedName: suggested,
                    types: [{ description: "Markdown",
                              accept: { "text/markdown": [".md"] } }],
                });
                window.__riparionFileHandle = handle;
            }
            if (handle) {
                const writable = await handle.createWritable();
                await writable.write(text);
                await writable.close();
                dioxus.send(handle.name);
                return;
            }
        } catch (e) {
            if (e?.name === "AbortError") { dioxus.send(null); return; }
            console.warn("save failed, falling back to download:", e);
        }
        // No File System Access API (or the write failed): download instead.
        const blob = new Blob([text], { type: "text/markdown" });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = suggested;
        document.body.appendChild(a);
        a.click();
        a.remove();
        URL.revokeObjectURL(url);
        dioxus.send(suggested);
        "#,
    );
    let _ = eval.send((content, suggested_name, force_picker));
    eval.recv::<Option<String>>().await.ok().flatten()
}

/// Turn one block's Markdown source into an [`Element`].
fn render_block(src: String) -> Element {
    // Atomic `[[embed]]` lines render as a distinct card so they read as a unit.
    let trimmed = src.trim();
    if trimmed.starts_with("[[") && trimmed.ends_with("]]") {
        let label = trimmed.trim_start_matches('[').trim_end_matches(']');
        return rsx! { div { class: "embed", "🔗 {label}" } };
    }

    // SECURITY: this demo renders the parser's HTML verbatim via
    // `dangerous_inner_html`, and `Options::all()` keeps raw inline HTML in the
    // output. That is fine for a local demo over trusted, self-authored content,
    // but it is an XSS vector for any untrusted input. A real app should run the
    // HTML through a sanitizer (e.g. `ammonia`) before injecting it.
    let mut out = String::new();
    let parser = Parser::new_ext(&src, Options::all());
    html::push_html(&mut out, parser);
    rsx! { div { dangerous_inner_html: "{out}" } }
}

/// Obsidian-style "Properties" card for the document's leading YAML
/// frontmatter block.
///
/// Scalar `key: value` lines edit inline (click the value), with add / remove
/// affordances. Everything else — sequences, nested maps, comments — shows
/// read-only; the `{}` YAML button (or any click outside a field) bubbles up to
/// the block's `on_activate`, swapping the whole block to the editor's normal
/// raw textarea as the escape hatch. Edits are line-targeted rewrites of the
/// raw YAML, so untouched lines (including comments and ordering) survive
/// byte-for-byte.
#[component]
fn FrontmatterCard(text: String, body: Signal<String>) -> Element {
    // Absolute YAML line index (0 = opening `---`) being edited inline.
    let editing = use_signal(|| None::<usize>);
    let draft = use_signal(String::new);
    // The "+ Add property" mini-form.
    let mut adding = use_signal(|| false);
    let mut new_key = use_signal(String::new);
    let mut new_value = use_signal(String::new);

    // The routing guard in `app` ensures `text` is a whole frontmatter run
    // (possibly with the trailing blank separator absorbed).
    let yaml_len = frontmatter_len(&text).unwrap_or(text.len());
    let lines: Vec<String> = text[..yaml_len]
        .lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();
    // Interior rows between the two delimiter lines, keyed by absolute index.
    let rows: Vec<(usize, String)> = lines
        .get(1..lines.len().saturating_sub(1))
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, l)| (i + 1, l.clone()))
        .collect();
    let empty = rows.is_empty();

    let mut commit_add = move || {
        fm_append_field(body, new_key.peek().as_str(), new_value.peek().as_str());
        adding.set(false);
    };

    rsx! {
        div { class: "fm-card",
            div { class: "fm-header",
                span { class: "fm-title", "Properties" }
                // No stop_propagation: the click bubbles to the block's
                // on_activate and swaps the card for the raw-YAML textarea.
                button { class: "fm-yaml-btn", title: "Edit the raw YAML", "{{}} YAML" }
            }
            for (line_idx , line) in rows {
                FrontmatterRow {
                    key: "{line_idx}:{line}",
                    line_idx,
                    line,
                    body,
                    editing,
                    draft,
                }
            }
            if empty && !adding() {
                div { class: "fm-row", span { class: "fm-empty", "No properties" } }
            }
            if adding() {
                div { class: "fm-row", onclick: move |e: MouseEvent| e.stop_propagation(),
                    input {
                        class: "fm-input",
                        placeholder: "key",
                        value: "{new_key}",
                        oninput: move |e: FormEvent| new_key.set(e.value()),
                        onmounted: move |e: MountedEvent| {
                            spawn(async move {
                                let _ = e.set_focus(true).await;
                            });
                        },
                        onkeydown: move |e: KeyboardEvent| {
                            if e.key() == Key::Enter {
                                e.prevent_default();
                                commit_add();
                            } else if e.key() == Key::Escape {
                                adding.set(false);
                            }
                        },
                    }
                    input {
                        class: "fm-input",
                        placeholder: "value",
                        value: "{new_value}",
                        oninput: move |e: FormEvent| new_value.set(e.value()),
                        onkeydown: move |e: KeyboardEvent| {
                            if e.key() == Key::Enter {
                                e.prevent_default();
                                commit_add();
                            } else if e.key() == Key::Escape {
                                adding.set(false);
                            }
                        },
                    }
                    button {
                        class: "fm-x",
                        title: "Cancel",
                        onclick: move |e: MouseEvent| {
                            e.stop_propagation();
                            adding.set(false);
                        },
                        "✕"
                    }
                }
            } else {
                button {
                    class: "fm-add",
                    onclick: move |e: MouseEvent| {
                        e.stop_propagation();
                        new_key.set(String::new());
                        new_value.set(String::new());
                        adding.set(true);
                    },
                    "+ Add property"
                }
            }
        }
    }
}

/// One interior line of the frontmatter, rendered as a card row.
///
/// Three shapes: a scalar `key: value` field edits inline; a parsed but
/// non-scalar field (`tags: [a, b]`, `key:` introducing a nested block) shows
/// locked; anything else (comments, indented continuation lines) shows as a raw
/// monospace line. Clicks on locked/raw rows bubble up and open the raw YAML.
#[component]
fn FrontmatterRow(
    line_idx: usize,
    line: String,
    body: Signal<String>,
    mut editing: Signal<Option<usize>>,
    mut draft: Signal<String>,
) -> Element {
    let Some((key, value)) = fm_parse_field(&line) else {
        return rsx! {
            div { class: "fm-row", span { class: "fm-raw", "{line}" } }
        };
    };

    if editing() == Some(line_idx) {
        // Inline value editor: Enter/blur commits the rewritten line, Esc
        // cancels. The commit is guarded on `editing` so Enter's blur (the
        // input unmounting) can't double-commit.
        let commit_key = key.clone();
        let commit = move || {
            if editing.peek().is_some() {
                let new_line = fm_line(&commit_key, draft.peek().trim());
                fm_rewrite_line(body, line_idx, Some(new_line));
                editing.set(None);
            }
        };
        let mut commit_b = commit.clone();
        return rsx! {
            div { class: "fm-row", onclick: move |e: MouseEvent| e.stop_propagation(),
                span { class: "fm-key", "{key}" }
                input {
                    class: "fm-input",
                    value: "{draft}",
                    oninput: move |e: FormEvent| draft.set(e.value()),
                    onmounted: move |e: MountedEvent| {
                        spawn(async move {
                            let _ = e.set_focus(true).await;
                        });
                    },
                    onkeydown: {
                        let mut commit = commit.clone();
                        move |e: KeyboardEvent| {
                            if e.key() == Key::Enter {
                                e.prevent_default();
                                commit();
                            } else if e.key() == Key::Escape {
                                editing.set(None);
                            }
                        }
                    },
                    onblur: move |_| commit_b(),
                }
            }
        };
    }

    if fm_scalar_editable(&value) {
        let display = value.clone();
        rsx! {
            div { class: "fm-row",
                span { class: "fm-key", "{key}" }
                span {
                    class: "fm-value",
                    title: "Click to edit",
                    onclick: move |e: MouseEvent| {
                        e.stop_propagation();
                        draft.set(value.clone());
                        editing.set(Some(line_idx));
                    },
                    "{display}"
                }
                button {
                    class: "fm-x",
                    title: "Remove property",
                    onclick: move |e: MouseEvent| {
                        e.stop_propagation();
                        fm_rewrite_line(body, line_idx, None);
                    },
                    "✕"
                }
            }
        }
    } else {
        rsx! {
            div { class: "fm-row",
                span { class: "fm-key", "{key}" }
                span { class: "fm-locked", title: "Edit via the {{}} YAML button", "{value}" }
            }
        }
    }
}

/// Parse a top-level `key: value` frontmatter line. The key must be unindented,
/// `[A-Za-z0-9_.-]+`, and followed by `:` plus whitespace (or end of line) —
/// so `http://…` continuation text never reads as a field.
fn fm_parse_field(line: &str) -> Option<(String, String)> {
    if line.starts_with(char::is_whitespace) {
        return None;
    }
    let (key, rest) = line.split_once(':')?;
    let key_ok = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if !key_ok || !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
        return None;
    }
    Some((key.to_string(), rest.trim().to_string()))
}

/// True when `value` is a plain scalar the card can edit inline. Flow
/// sequences/maps, anchors, block scalars and empty values (which may introduce
/// a nested block) stay read-only — the raw-YAML escape hatch edits those.
fn fm_scalar_editable(value: &str) -> bool {
    !value.is_empty() && !value.starts_with(['[', '{', '&', '*', '|', '>', '#'])
}

/// Render a `key: value` line (just `key:` when the value is empty).
fn fm_line(key: &str, value: &str) -> String {
    if value.is_empty() {
        format!("{key}:")
    } else {
        format!("{key}: {value}")
    }
}

/// Rewrite (`Some`) or remove (`None`) one interior line of the document's
/// leading frontmatter, leaving every other byte of the document untouched.
/// The delimiter lines are never touched.
fn fm_rewrite_line(mut body: Signal<String>, line_idx: usize, new_line: Option<String>) {
    let doc = body.peek().clone();
    let Some(len) = frontmatter_len(&doc) else {
        return;
    };
    let mut lines: Vec<String> = doc[..len]
        .lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();
    if line_idx == 0 || line_idx + 1 >= lines.len() {
        return;
    }
    match new_line {
        Some(l) => lines[line_idx] = l,
        None => {
            lines.remove(line_idx);
        }
    }
    body.set(format!("{}\n{}", lines.join("\n"), &doc[len..]));
}

/// Append a `key: value` field just above the closing delimiter of the
/// document's leading frontmatter. Invalid keys are dropped silently (demo).
fn fm_append_field(mut body: Signal<String>, key: &str, value: &str) {
    let key = key.trim();
    let key_ok = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if !key_ok {
        return;
    }
    let doc = body.peek().clone();
    let Some(len) = frontmatter_len(&doc) else {
        return;
    };
    let mut lines: Vec<String> = doc[..len]
        .lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();
    if lines.len() < 2 {
        return;
    }
    let at = lines.len() - 1;
    lines.insert(at, fm_line(key, value.trim()));
    body.set(format!("{}\n{}", lines.join("\n"), &doc[len..]));
}
