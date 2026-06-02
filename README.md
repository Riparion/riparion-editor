# riparion-editor

A **block-swap live-preview** editor for [Dioxus](https://dioxuslabs.com) 0.7.

A document renders as a column of styled blocks; the block you're editing swaps
in place to a raw `<textarea>`, and re-renders when you leave it. Because every
editable element is an ordinary controlled `<textarea>`, there's no
`contenteditable`-vs-virtual-DOM caret fight — no JavaScript and no rich-text
engine. The trade-off: inline formatting doesn't render *within* the line you're
currently typing on (that's the `contenteditable`-only trick). Markdown is the
typical use, but the crate is **renderer-agnostic** — you inject the renderer.

## Features

- **Block-swap editing** — rendered blocks, with the active one as a textarea.
- **Bring-your-own renderer** — you pass `render_block` (and optionally
  `is_atomic`); the crate pins no markdown dialect, sanitizer, or embed syntax.
- **Lossless block model** — `split_blocks` / `join_blocks` round-trip the source
  byte-for-byte (respecting fenced code and atomic lines).
- **Drag-to-reorder** blocks (via [`riparion-dnd`](https://github.com/Riparion/riparion-dnd)).
- **Keyboard editing** (web): double-Enter starts a new block with the caret in
  it; cross-block arrow navigation (column-preserving); selection formatting —
  type `*` `_` `` ` `` `~` to wrap, plus `Cmd/Ctrl+B` / `+I` / `+K` (link).
- **Auto-growing** textarea; a discoverable **+ New block** affordance.

## Usage

```rust
use dioxus::prelude::*;
use riparion_editor::BlockEditor;

#[component]
fn Editor(body: Signal<String>) -> Element {
    rsx! {
        BlockEditor {
            body, // canonical source; the single source of truth
            // Turn one block's source into an Element — plug in your renderer
            // (pulldown-cmark, comrak, …). This is also where embeds get mounted.
            render_block: Callback::new(|src: String| rsx! { div { "{src}" } }),
            // Optional: a line for which this returns true becomes its own block
            // (e.g. a custom `[[embed]]`), so editing it touches just that line.
            is_atomic: None,
            block_class: "prose".into(),     // class for a rendered block
            textarea_class: "w-full font-mono".into(), // class for the active textarea
        }
    }
}
```

`BlockEditor` props:

| Prop | Type | Notes |
|---|---|---|
| `body` | `Signal<String>` | Canonical source, spliced in place; your save path is unchanged. |
| `render_block` | `Callback<String, Element>` | Renders one block's source. |
| `is_atomic` | `Option<Callback<String, bool>>` | Marks single-line atomic blocks (e.g. embeds). |
| `block_class` | `String` | Class for an inactive rendered block. |
| `textarea_class` | `String` | Class for the active-block `<textarea>`. |

Public helpers (`split_blocks`, `join_blocks`, `rejoin_normalized`,
`normalize_body`) are exposed for hosts that want to operate on the block model
directly.

## Features flag

Enable `web` for the browser build — it pulls in `web-sys` / `wasm-bindgen` for
the caret/selection mechanics (and turns on `riparion-dnd/web`):

```toml
[dependencies]
riparion-editor = { version = "0.1", features = ["web"] }
```

Off the web target the keyboard/selection helpers compile out; the editor still
renders and click-to-edit works.

## Running the example

[`examples/markdown.rs`](examples/markdown.rs) is a complete browser demo: it
wires [`pulldown-cmark`](https://crates.io/crates/pulldown-cmark) into
`render_block`, seeds a document, and treats `[[embed]]` lines as atomic blocks.
Run it with the [Dioxus CLI](https://dioxuslabs.com/learn/0.7/CLI/):

```sh
dx serve --example markdown --features web
```

Then open the printed URL and click any block to edit its raw Markdown.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option.
