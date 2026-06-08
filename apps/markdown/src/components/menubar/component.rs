use dioxus::prelude::*;
use dioxus_primitives::menubar::{
    self, MenubarContentProps, MenubarItemProps, MenubarMenuProps, MenubarProps,
    MenubarTriggerProps,
};
// The Asset is declared separately and pinned with an explicit
// `document::Stylesheet` (here and from main.rs): `#[css_module]`'s own
// OnceLock-driven <link> injection silently drops on remounts, leaving the
// widget unstyled.
pub(crate) const MENUBAR_CSS: Asset = asset!(
    "/src/components/menubar/style.css",
    AssetOptions::css_module()
);

#[css_module("/src/components/menubar/style.css")]
struct Styles;

#[component]
pub fn Menubar(props: MenubarProps) -> Element {
    rsx! {
        document::Stylesheet { href: MENUBAR_CSS }
        menubar::Menubar {
            class: Styles::dx_menubar,
            disabled: props.disabled,
            roving_loop: props.roving_loop,
            attributes: props.attributes,
            {props.children}
        }
    }
}

#[component]
pub fn MenubarMenu(props: MenubarMenuProps) -> Element {
    rsx! {
        menubar::MenubarMenu {
            class: Styles::dx_menubar_menu,
            index: props.index,
            disabled: props.disabled,
            attributes: props.attributes,
            {props.children}
        }
    }
}

#[component]
pub fn MenubarTrigger(props: MenubarTriggerProps) -> Element {
    rsx! {
        menubar::MenubarTrigger { class: Styles::dx_menubar_trigger, attributes: props.attributes, {props.children} }
    }
}

#[component]
pub fn MenubarContent(props: MenubarContentProps) -> Element {
    rsx! {
        menubar::MenubarContent {
            class: Styles::dx_menubar_content,
            id: props.id,
            attributes: props.attributes,
            {props.children}
        }
    }
}

#[component]
pub fn MenubarItem(props: MenubarItemProps) -> Element {
    rsx! {
        menubar::MenubarItem {
            class: Styles::dx_menubar_item,
            index: props.index,
            value: props.value,
            disabled: props.disabled,
            on_select: props.on_select,
            attributes: props.attributes,
            {props.children}
        }
    }
}
