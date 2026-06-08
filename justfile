# riparion-editor — block-swap live-preview editor for Dioxus.
# `just` with no recipe lists them all.
default:
    @just --list

# Serve the riparion-mdedit demo app in a browser (Dioxus CLI, web target).
run:
    dx serve --package riparion-mdedit

# Same, but reachable from other devices on the LAN.
run-host:
    dx serve --package riparion-mdedit --addr 0.0.0.0

# Format check + host clippy + tests + wasm clippy — mirrors CI.
check:
    cargo fmt --all -- --check
    cargo clippy --no-default-features --all-targets -- -D warnings
    cargo test --no-default-features
    cargo clippy --no-default-features --features web --target wasm32-unknown-unknown -- -D warnings
    cargo clippy -p riparion-mdedit --target wasm32-unknown-unknown -- -D warnings
