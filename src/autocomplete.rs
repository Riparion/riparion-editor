//! Inline autocomplete for a trigger token typed inside a block's `<textarea>`.
//!
//! The crate stays app-agnostic: it knows the *trigger* (`[[/`) and the
//! mechanics (detect → filter → splice at the caret), but not what the
//! candidates mean. The host injects a `complete` callback that, given the query
//! typed after `[[/`, returns the [`CompletionItem`]s to offer; each item's
//! [`insert`](CompletionItem::insert) is the literal text spliced in on accept.
//!
//! Three layers, smallest first:
//! * pure logic — [`find_trigger`] / [`apply_completion`] (no `web` feature;
//!   unit-tested),
//! * the [`use_autocomplete`] hook — the single behavior source, reused by this
//!   crate's [`BlockEditor`](crate::BlockEditor) (Live mode) and by a host's own
//!   textarea (e.g. a raw source view), and
//! * [`CompletionPopup`] — themeless markup the caller positions and styles.
//!
//! The popup is driven entirely from the active textarea's `oninput` (detect)
//! and `onkeydown` (navigate / accept / dismiss); off `web` the hook is inert.

use dioxus::prelude::*;

/// One offered completion. `label` + `detail` are display-only; `insert` is the
/// literal text spliced in at the caret when the item is accepted.
#[derive(Debug, Clone, PartialEq)]
pub struct CompletionItem {
    pub label: String,
    pub detail: String,
    pub insert: String,
}

/// A `[[/…` trigger found before the caret: the byte offset of its opening `[`
/// and the `[A-Za-z0-9_-]*` query typed after `[[/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerMatch {
    /// Byte offset of the first `[` of `[[/` in the source.
    pub start: usize,
    /// The run after `[[/`, up to the caret (may be empty just after `[[/`).
    pub query: String,
}

/// True for the characters allowed in an embed name (`[[/name …]]`): the query
/// only stays "open" while it's one of these, so a space or `]` closes it.
fn is_query_char(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_'
}

/// Find a `[[/` trigger immediately before `caret_byte`, on the caret's own
/// line. Returns `None` unless the run between `[[/` and the caret is a valid
/// (possibly empty) embed-name query — so typing a space, `]`, or a newline
/// closes the trigger. `caret_byte` must be a char boundary.
pub fn find_trigger(text: &str, caret_byte: usize) -> Option<TriggerMatch> {
    let caret = caret_byte.min(text.len());
    let line_start = text[..caret].rfind('\n').map(|n| n + 1).unwrap_or(0);
    let seg = &text[line_start..caret];
    let rel = seg.rfind("[[/")?;
    let query = &seg[rel + 3..];
    if !query.chars().all(is_query_char) {
        return None;
    }
    Some(TriggerMatch {
        start: line_start + rel,
        query: query.to_string(),
    })
}

/// Replace `text[trigger.start..caret_byte]` with `insert`, forcing the snippet
/// onto its own line (a `\n` is added before/after only when non-newline text
/// abuts it) so the result is recognized as a standalone embed line. Returns the
/// new text and the new caret byte offset (the end of the inserted snippet).
pub fn apply_completion(
    text: &str,
    trigger: &TriggerMatch,
    caret_byte: usize,
    insert: &str,
) -> (String, usize) {
    let caret = caret_byte.min(text.len()).max(trigger.start);
    let prefix = &text[..trigger.start];
    let suffix = &text[caret..];
    let lead = if !prefix.is_empty() && !prefix.ends_with('\n') {
        "\n"
    } else {
        ""
    };
    let trail = if !suffix.is_empty() && !suffix.starts_with('\n') {
        "\n"
    } else {
        ""
    };
    let new = format!("{prefix}{lead}{insert}{trail}{suffix}");
    let new_caret = prefix.len() + lead.len() + insert.len();
    (new, new_caret)
}

/// What [`Autocomplete::on_keydown`] did with a key, so the caller knows whether
/// to run its own handling.
pub enum KeyOutcome {
    /// The popup was closed/inactive; the caller should handle the key normally.
    Ignored,
    /// The key drove the popup (navigate/dismiss) and was consumed.
    Consumed,
    /// An item was accepted; the caller must commit `value` to its source signal.
    Accepted { value: String },
}

/// Autocomplete state + behavior for one textarea. Cheap to copy (all signals);
/// created by [`use_autocomplete`]. Drive it from the textarea's `oninput`
/// ([`on_input`](Self::on_input)) and `onkeydown`
/// ([`on_keydown`](Self::on_keydown)), and render [`CompletionPopup`] from
/// [`open`](Self::open) / [`items`](Self::items) / [`selected`](Self::selected).
#[derive(Clone, Copy)]
pub struct Autocomplete {
    // Read only by the web-gated `on_input`; off `web` the hook is inert.
    #[cfg_attr(not(feature = "web"), allow(dead_code))]
    complete: Option<Callback<String, Vec<CompletionItem>>>,
    open: Signal<bool>,
    items: Signal<Vec<CompletionItem>>,
    /// Highlighted index, or `None` when the popup is open but the caret is
    /// still "in the text" — the first ↓/↑ moves the selection *into* the list.
    selected: Signal<Option<usize>>,
    /// The textarea element, captured on `oninput`, so a popup *click* (whose
    /// event target is the menu, not the textarea) can still splice into it.
    #[cfg(feature = "web")]
    el: Signal<Option<web_sys::HtmlTextAreaElement>>,
}

impl Autocomplete {
    /// Whether the popup is currently open.
    pub fn open(&self) -> bool {
        (self.open)()
    }
    /// The current candidate list.
    pub fn items(&self) -> Vec<CompletionItem> {
        (self.items)()
    }
    /// The highlighted index into [`items`](Self::items), or `None` while the
    /// caret is still in the text (before the first ↓/↑ enters the list).
    pub fn selected(&self) -> Option<usize> {
        (self.selected)()
    }

    /// Close the popup and clear its state. Called only from the web-gated
    /// input/keydown paths.
    #[cfg_attr(not(feature = "web"), allow(dead_code))]
    fn dismiss(mut self) {
        self.open.set(false);
        self.items.set(Vec::new());
        self.selected.set(None);
    }

    /// Detect/refresh the trigger after the textarea's value changed. Call from
    /// `oninput`, after committing the new value to the source signal.
    pub fn on_input(self, e: &FormEvent) {
        #[cfg(feature = "web")]
        {
            let Some(complete) = self.complete else {
                return;
            };
            let Some((ta, value, caret)) = formevent_textarea(e) else {
                self.dismiss();
                return;
            };
            let mut el = self.el;
            el.set(Some(ta));
            match find_trigger(&value, caret) {
                Some(t) => {
                    let items = complete.call(t.query);
                    if items.is_empty() {
                        self.dismiss();
                    } else {
                        let mut its = self.items;
                        let mut sel = self.selected;
                        let mut op = self.open;
                        its.set(items);
                        // Open with nothing highlighted: the first ↓/↑ enters the
                        // list. Re-filtering (typing) likewise resets to "in text".
                        sel.set(None);
                        op.set(true);
                    }
                }
                None => self.dismiss(),
            }
        }
        #[cfg(not(feature = "web"))]
        let _ = e;
    }

    /// Handle a keydown while the popup may be open. Call **first** in the
    /// textarea's `onkeydown` and branch on the result: `Accepted` → commit the
    /// value; `Consumed` → return; `Ignored` → run normal key handling.
    pub fn on_keydown(self, e: &KeyboardEvent) -> KeyOutcome {
        #[cfg(feature = "web")]
        {
            if !(self.open)() {
                return KeyOutcome::Ignored;
            }
            let len = (self.items)().len();
            if len == 0 {
                return KeyOutcome::Ignored;
            }
            let mut sel = self.selected;
            match e.key() {
                // ↓/↑ move *into* the list (first press) then navigate it. While
                // open they always belong to the popup, never the textarea caret.
                Key::ArrowDown => {
                    e.prevent_default();
                    sel.set(Some(match sel() {
                        None => 0,
                        Some(i) => (i + 1) % len,
                    }));
                    KeyOutcome::Consumed
                }
                Key::ArrowUp => {
                    e.prevent_default();
                    sel.set(Some(match sel() {
                        None => len - 1,
                        Some(i) => (i + len - 1) % len,
                    }));
                    KeyOutcome::Consumed
                }
                // Enter/Tab accept *only* once an item is selected; with nothing
                // selected they fall through (a normal newline / tab in the text).
                Key::Enter | Key::Tab => match sel() {
                    Some(i) => {
                        e.prevent_default();
                        match self.accept(i) {
                            Some(value) => KeyOutcome::Accepted { value },
                            None => {
                                self.dismiss();
                                KeyOutcome::Consumed
                            }
                        }
                    }
                    None => KeyOutcome::Ignored,
                },
                // Esc closes the menu with no insert; editing resumes as normal.
                Key::Escape => {
                    e.prevent_default();
                    self.dismiss();
                    KeyOutcome::Consumed
                }
                // Any other key (typing more of the query) falls through to the
                // textarea; `on_input` then re-filters.
                _ => KeyOutcome::Ignored,
            }
        }
        #[cfg(not(feature = "web"))]
        {
            let _ = e;
            KeyOutcome::Ignored
        }
    }

    /// Splice the item at `index` into the captured textarea at the live caret,
    /// move the DOM caret past it, close the popup, and return the new value for
    /// the caller to commit. `None` if there's no live trigger anymore.
    #[cfg(feature = "web")]
    pub fn accept(self, index: usize) -> Option<String> {
        let items = (self.items)();
        let item = items.get(index)?;
        let ta = (self.el)()?;
        let value = ta.value();
        let caret_u16 = ta.selection_start().ok().flatten()? as usize;
        let caret = crate::utf16_to_byte(&value, caret_u16);
        let trigger = find_trigger(&value, caret)?;
        let (new, new_caret) = apply_completion(&value, &trigger, caret, &item.insert);
        // Set the DOM value first so the controlled re-render writes an identical
        // string (no caret reset), then place the caret past the snippet.
        ta.set_value(&new);
        let u = crate::byte_to_utf16(&new, new_caret);
        let _ = ta.set_selection_range(u, u);
        self.dismiss();
        Some(new)
    }
}

/// Per-textarea autocomplete state. `complete` maps the typed query to
/// candidates; `None` disables autocomplete (the hook stays inert). Off `web`
/// the returned [`Autocomplete`] never opens.
pub fn use_autocomplete(complete: Option<Callback<String, Vec<CompletionItem>>>) -> Autocomplete {
    Autocomplete {
        complete,
        open: use_signal(|| false),
        items: use_signal(Vec::new),
        selected: use_signal(|| None),
        #[cfg(feature = "web")]
        el: use_signal(|| None),
    }
}

/// Read `(textarea, value, caret_byte)` from an input event fired by a textarea.
#[cfg(feature = "web")]
fn formevent_textarea(e: &FormEvent) -> Option<(web_sys::HtmlTextAreaElement, String, usize)> {
    use dioxus::web::WebEventExt;
    use wasm_bindgen::JsCast;
    let we: web_sys::Event = e.data().try_as_web_event()?;
    let ta = we
        .target()?
        .dyn_into::<web_sys::HtmlTextAreaElement>()
        .ok()?;
    let value = ta.value();
    let start = ta.selection_start().ok().flatten()? as usize;
    let byte = crate::utf16_to_byte(&value, start);
    Some((ta, value, byte))
}

/// A themeless completion menu the caller positions (render it inside a
/// `position: relative` wrapper around the textarea) and styles via the
/// `*_class` props. Items fire on `mousedown` (with `prevent_default`) so
/// picking one doesn't blur the textarea before the splice lands.
#[component]
pub fn CompletionPopup(
    items: Vec<CompletionItem>,
    /// The highlighted item, or `None` when no item is selected yet.
    selected: Option<usize>,
    on_pick: EventHandler<usize>,
    #[props(default)] on_hover: Option<EventHandler<usize>>,
    /// Class for the menu container.
    #[props(default)]
    menu_class: String,
    /// Class for each item row.
    #[props(default)]
    item_class: String,
    /// Class added to the highlighted item row.
    #[props(default)]
    item_active_class: String,
) -> Element {
    if items.is_empty() {
        return rsx! {};
    }
    rsx! {
        div {
            // Self-contained positioning so the menu is usable with zero host
            // CSS; colors/spacing come from `menu_class`.
            style: "position:absolute;left:0;top:100%;z-index:50;margin-top:0.25rem;max-height:18rem;overflow-y:auto;",
            class: "{menu_class}",
            for (i , item) in items.iter().enumerate() {
                button {
                    key: "{item.label}",
                    r#type: "button",
                    class: if Some(i) == selected { format!("{item_class} {item_active_class}") } else { item_class.clone() },
                    style: "display:block;width:100%;text-align:left;cursor:pointer;font:inherit;color:inherit;background:transparent;border:0;",
                    // mousedown keeps focus on the textarea (a plain click would
                    // fire after its blur had torn the active editor down).
                    onmousedown: move |e: MouseEvent| {
                        e.prevent_default();
                        on_pick.call(i);
                    },
                    onmouseenter: move |_| {
                        if let Some(h) = on_hover {
                            h.call(i);
                        }
                    },
                    div { style: "font-size:0.875rem;font-weight:500;", "{item.label}" }
                    if !item.detail.is_empty() {
                        div { style: "font-size:0.75rem;opacity:0.6;", "{item.detail}" }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_just_opened() {
        let t = find_trigger("[[/", 3).unwrap();
        assert_eq!(t.start, 0);
        assert_eq!(t.query, "");
    }

    #[test]
    fn trigger_mid_line_with_query() {
        let t = find_trigger("ab [[/cou", 9).unwrap();
        assert_eq!(t.start, 3);
        assert_eq!(t.query, "cou");
    }

    #[test]
    fn trigger_stops_before_closing_brackets() {
        // Caret sits right after the name, before `]]` — still a live trigger.
        let t = find_trigger("[[/cou]]", 6).unwrap();
        assert_eq!(t.query, "cou");
    }

    #[test]
    fn no_trigger_after_space_or_bracket() {
        assert!(find_trigger("[[/cou ", 7).is_none());
        assert!(find_trigger("[[/co]", 6).is_none());
    }

    #[test]
    fn no_trigger_across_newline() {
        // The `[[/` is on the previous line; the caret's line has no trigger.
        assert!(find_trigger("[[/\nfoo", 7).is_none());
    }

    #[test]
    fn trigger_handles_multibyte_prefix() {
        let text = "café [[/c"; // "café " is 6 bytes (é = 2)
        let t = find_trigger(text, text.len()).unwrap();
        assert_eq!(t.start, 6);
        assert_eq!(t.query, "c");
    }

    #[test]
    fn apply_at_line_start_no_extra_newlines() {
        let t = find_trigger("[[/cou", 6).unwrap();
        let (new, caret) = apply_completion("[[/cou", &t, 6, "[[/counter]]");
        assert_eq!(new, "[[/counter]]");
        assert_eq!(caret, "[[/counter]]".len());
    }

    #[test]
    fn apply_forces_own_line_after_prose() {
        let text = "hello [[/cou";
        let t = find_trigger(text, text.len()).unwrap();
        let (new, caret) = apply_completion(text, &t, text.len(), "[[/c]]");
        assert_eq!(new, "hello \n[[/c]]");
        assert_eq!(caret, new.len());
    }

    #[test]
    fn apply_forces_own_line_before_suffix() {
        let text = "[[/cou after";
        let t = find_trigger(text, 6).unwrap();
        let (new, _caret) = apply_completion(text, &t, 6, "[[/c]]");
        assert_eq!(new, "[[/c]]\n after");
    }
}
