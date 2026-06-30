# ArcFlow

ArcFlow is an open-source cross-platform client for compatible electrostimulation devices.

The project is being built as a Rust-first core with React frontends. Desktop and mobile targets use Tauri 2, while Bluetooth, protocol handling, wave generation, plugin runtime, storage, and synchronization live in Rust crates.

## Current workspace

```text
crates/
  protocol/      Bluetooth protocol frames and helpers.

docs/
  agents/        Agent setup notes.
  protocol/      Protocol implementation notes.
```

Run the current Rust checks with:

```sh
cargo test -p arcflow-protocol
```
