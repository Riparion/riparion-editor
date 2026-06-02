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
use riparion_editor::BlockEditor;

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

    let mut out = String::new();
    let parser = Parser::new_ext(&src, Options::all());
    html::push_html(&mut out, parser);
    rsx! { div { dangerous_inner_html: "{out}" } }
}
