//! A runnable [`BlockEditor`] demo with a real Markdown renderer.
//!
//! It plugs [`pulldown-cmark`] into the crate's `render_block` callback, so each
//! inactive block renders as HTML and the block you click swaps to a raw
//! `<textarea>`. Lines that look like `[[embed]]` are marked atomic via
//! `is_atomic` and render as a standalone card.
//!
//! A menubar above the editor (dx catalog `menubar` + `alert_dialog`, installed
//! with `dx components add --module-path examples/markdown/components …`) adds
//! File ▸ New (confirm, then clear), File ▸ Save (download as `.md`), and an
//! About dialog.
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
use components::menubar::{Menubar, MenubarContent, MenubarItem, MenubarMenu, MenubarTrigger};
use components::switch::Switch;
use dioxus::prelude::*;
use pulldown_cmark::{html, Options, Parser};
use riparion_editor::{BlockEditor, CompletionItem};

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

/// Seed document, exercising headings, lists, a fenced code block, and an
/// atomic `[[embed]]` line.
const SEED: &str = r#"# riparion-editor

A **block-swap** live-preview editor. Click any block to edit its raw Markdown;
click away (or press Esc) to render it again.

## Try it

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
        "save" => save_markdown(body.peek().as_str()),
        "about" => about_open.set(true),
        _ => {}
    };

    rsx! {
        document::Style { {STYLES} }
        // Design tokens (colors, radii, focus rings) shared by the dx catalog
        // widgets under `components/`.
        document::Link { rel: "stylesheet", href: asset!("/assets/dx-components-theme.css") }
        div { class: "page",
            div { class: "toolbar",
                Menubar {
                    MenubarMenu { index: 0usize,
                        MenubarTrigger { "File" }
                        MenubarContent {
                            MenubarItem { index: 0usize, value: "new", on_select: on_menu, "New" }
                            MenubarItem { index: 1usize, value: "save", on_select: on_menu, "Save" }
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
                render_block: Callback::new(render_block),
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
                    on_click: move |_| body.set(String::new()),
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

/// `File ▸ Save` — hand the current Markdown source to the browser as a
/// `document.md` download (a wasm app has no direct disk access, so "save to
/// local disk" means a Blob + temporary object-URL anchor click).
fn save_markdown(content: &str) {
    let eval = document::eval(
        r#"
        const text = await dioxus.recv();
        const blob = new Blob([text], { type: "text/markdown" });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = "document.md";
        document.body.appendChild(a);
        a.click();
        a.remove();
        URL.revokeObjectURL(url);
        "#,
    );
    let _ = eval.send(content);
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
