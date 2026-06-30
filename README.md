# ArcFlow

ArcFlow is an open-source cross-platform client for compatible electrostimulation devices.

The project is being built as a Rust-first core with React frontends. Desktop and mobile targets use Tauri 2, while Bluetooth, protocol handling, wave generation, plugin runtime, storage, and synchronization live in Rust crates.

## Current workspace

```text
apps/
  desktop/      Tauri 2 + React desktop app; also seeds mobile targets.
  mobile/       Mobile target notes for the shared Tauri 2 app.

crates/
  core/          Core orchestration types and Rust-only device control surface.
  external-control/
                 WebSocket external-control protocol messages.
  plugin-runtime/
                 WASM/JS plugin manifest and capability model.
  protocol/      Bluetooth protocol frames and helpers.
  storage/       SQLite storage owned by Rust.
  wave/          Safe wave plans and protocol conversion.

docs/
  agents/        Agent setup notes.
  protocol/      Protocol implementation notes.
```

Run the current Rust checks with:

```sh
cargo test --workspace
```

Run the desktop frontend build with:

```sh
pnpm install
pnpm check:frontend
```
