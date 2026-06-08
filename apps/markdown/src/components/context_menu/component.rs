use dioxus::prelude::*;
use dioxus_primitives::context_menu::{
    self, ContextMenuContentProps, ContextMenuItemProps, ContextMenuProps, ContextMenuTriggerProps,
};

// Asset pinned via explicit `document::Stylesheet` — see menubar/component.rs.
pub(crate) const CONTEXT_MENU_CSS: Asset = asset!(
    "/src/components/context_menu/style.css",
    AssetOptions::css_module()
);

#[css_module("/src/components/context_menu/style.css")]
struct Styles;

#[component]
pub fn ContextMenu(props: ContextMenuProps) -> Element {
    rsx! {
        document::Stylesheet { href: CONTEXT_MENU_CSS }
        context_menu::ContextMenu {
            disabled: props.disabled,
            open: props.open,
            default_open: props.default_open,
            on_open_change: props.on_open_change,
            roving_loop: props.roving_loop,
            attributes: props.attributes,
            {props.children}
        }
    }
}

#[component]
pub fn ContextMenuTrigger(props: ContextMenuTriggerProps) -> Element {
    rsx! {
        // Upstream ships demo-only inline styles here (padding, dashed border,
        // primary background, centered text); stripped so the trigger wraps the
        // editor's rendered blocks without restyling them.
        context_menu::ContextMenuTrigger {
            attributes: props.attributes,
            {props.children}
        }
    }
}

#[component]
pub fn ContextMenuContent(props: ContextMenuContentProps) -> Element {
    rsx! {
        context_menu::ContextMenuContent {
            class: Styles::dx_context_menu_content,
            id: props.id,
            attributes: props.attributes,
            {props.children}
        }
    }
}

#[component]
pub fn ContextMenuItem(props: ContextMenuItemProps) -> Element {
    rsx! {
        context_menu::ContextMenuItem {
            class: Styles::dx_context_menu_item,
            disabled: props.disabled,
            value: props.value,
            index: props.index,
            on_select: props.on_select,
            attributes: props.attributes,
            {props.children}
        }
    }
}
