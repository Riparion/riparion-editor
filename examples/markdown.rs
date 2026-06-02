//! A runnable [`BlockEditor`] demo with a real Markdown renderer.
//!
//! It plugs [`pulldown-cmark`] into the crate's `render_block` callback, so each
//! inactive block renders as HTML and the block you click swaps to a raw
//! `<textarea>`. Lines that look like `[[embed]]` are marked atomic via
//! `is_atomic` and render as a standalone card.
//!
//! Run it in a browser with the Dioxus CLI (the `web` feature pulls in the
//! caret/selection/drag mechanics):
//!
//! ```sh
//! dx serve --example markdown --features web
//! ```

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
body { margin: 0; background: #f6f7f9; color: #1a1a1a;
    font: 16px/1.6 system-ui, -apple-system, sans-serif; }
.page { max-width: 44rem; margin: 0 auto; padding: 2rem 1rem 6rem; }
.prose { padding: 0.15rem 0.4rem; border-radius: 0.35rem; }
.prose:hover { background: #eceef1; }
.prose pre { background: #1e2430; color: #e7eaf0; padding: 0.9rem 1rem;
    border-radius: 0.5rem; overflow-x: auto; }
.prose code { font-family: ui-monospace, SFMono-Regular, monospace; font-size: 0.9em; }
.prose pre code { background: none; padding: 0; }
.embed { border: 1px dashed #9aa3af; border-radius: 0.5rem; padding: 0.75rem 1rem;
    background: #fff; color: #4b5563; font-style: italic; }
.editor-textarea { width: 100%; box-sizing: border-box; padding: 0.4rem 0.5rem;
    border: 1px solid #c7ccd3; border-radius: 0.4rem; background: #fff;
    font-family: ui-monospace, SFMono-Regular, monospace; font-size: 0.95rem; }
.ac-menu { min-width: 18rem; background: #fff; border: 1px solid #c7ccd3;
    border-radius: 0.5rem; box-shadow: 0 8px 24px rgba(0,0,0,0.12); padding: 0.25rem; }
.ac-item { padding: 0.35rem 0.6rem; border-radius: 0.35rem; }
.ac-item:hover { background: #eceef1; }
.ac-item-active { background: #e3effe; }
"#;

fn main() {
    dioxus::launch(app);
}

fn app() -> Element {
    let body = use_signal(|| SEED.to_string());

    rsx! {
        document::Style { {STYLES} }
        div { class: "page",
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
    }
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
