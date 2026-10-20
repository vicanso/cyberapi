# Cyber API

A fast, native HTTP API client built with Rust and [GPUI](https://www.gpui.rs/), from one codebase for the desktop and the browser.

## Desktop

```bash
make dev
```

## Browser

Needs the nightly toolchain with the `wasm32-unknown-unknown` target (picked up from `cyberapi-web/rust-toolchain.toml`) and `wasm-pack` (`cargo install wasm-pack`).

```bash
make web-bundle
make web-serve   # http://127.0.0.1:7380/
```

## License

Apache-2.0
