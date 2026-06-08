use dioxus::prelude::*;
use dioxus_primitives::switch::{self, SwitchProps};

// Asset pinned via explicit `document::Stylesheet` — see menubar/component.rs.
pub(crate) const SWITCH_CSS: Asset = asset!(
    "/src/components/switch/style.css",
    AssetOptions::css_module()
);

#[css_module("/src/components/switch/style.css")]
struct Styles;

#[component]
pub fn Switch(props: SwitchProps) -> Element {
    rsx! {
        document::Stylesheet { href: SWITCH_CSS }
        switch::Switch {
            class: Styles::dx_switch,
            checked: props.checked,
            default_checked: props.default_checked,
            disabled: props.disabled,
            required: props.required,
            name: props.name,
            value: props.value,
            on_checked_change: props.on_checked_change,
            attributes: props.attributes,
            switch::SwitchThumb { class: Styles::dx_switch_thumb }
        }
    }
}
