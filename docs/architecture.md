# ArcFlow Architecture

ArcFlow is a Rust-first, plugin-first client for compatible electrostimulation
devices. Desktop and mobile shells use Tauri 2 with a React UI; Rust owns device
access, safety, storage, protocol handling, plugins, scripts, and external
control.

## Runtime Shape

```mermaid
flowchart TD
  React["React + TypeScript UI"]
  IPC["Tauri 2 IPC"]
  Core["arcflow-core"]
  Platform["arcflow-tauri-platform"]
  BLE["Platform BLE"]
  Device["Coyote device"]
  Plugins["WASM/JS plugins"]
  WS["External WebSocket clients"]
  Storage["SQLite storage"]

  React --> IPC --> Core
  Core --> Platform --> BLE --> Device
  Core --> Storage
  Plugins --> Core
  WS --> Core
```

React does not call Bluetooth, SQLite, or plugin runtimes directly. UI commands
cross Tauri IPC into Rust. Plugins and external WebSocket clients also route
through Rust Core and capability checks before any device-facing operation.

## Workspace Crates

- `crates/core`: orchestration, safety limits, BLE seams, Coyote command
  builders, script execution, plugin API bridge, and external request routing.
- `crates/tauri-platform`: Tauri 2 desktop/mobile platform adapters. The current
  BLE scaffold exposes unsupported, permission-denied, powered-off, and ready
  scan states.
- `crates/protocol`: byte-level Coyote V2/V3 protocol parsing and command
  construction. It does not manage Bluetooth connections.
- `crates/wave`: safe wave-domain values and Coyote V3 window conversion.
- `crates/script`: safe script document model and compiler.
- `crates/plugin-runtime`: WASM/JavaScript plugin manifests, capabilities,
  sandbox policy, registry, runtime routing, and a deterministic recording
  runtime used until real WASM/JS engines are attached.
- `crates/storage`: SQLite schema and Rust-owned stores for plugin data,
  installed plugins, and scripts.
- `crates/external-control`: local WebSocket protocol and gateway.

## Device Flow

Platform BLE adapters emit advertisements and transport operations into Rust.
Core maps Coyote V2/V3 advertisements into `DeviceStatus`, builds safe Coyote V3
writes through `CoyoteV3CommandBuilder`, and sends bytes through `BleTransport`.

Bluetooth logic must stay in Rust. The future web target may add a separate Web
Bluetooth implementation, but desktop and mobile use Tauri 2 and Rust.

## Plugin And External Control

Plugins support `wasm` and `javascript` runtimes. They run behind the Plugin API
and never receive direct Bluetooth access. External software connects through the
local WebSocket gateway and receives only the capabilities granted during hello.
Plugin manifests must point WASM runtimes at `.wasm` entries and JavaScript
runtimes at `.js` or `.mjs` entries.

The current runtime implementation includes a recording adapter for both WASM
and JavaScript. It validates sandbox admission, records load/invoke/unload
events, and returns empty plugin output so host wiring can be tested without
executing untrusted code. Real engines will replace the adapter behind the same
`RuntimeAdapter` boundary.

Desktop startup restores the persisted plugin registry into this sandboxed
runtime host. UI and WebSocket plugin registry mutations update SQLite first,
then synchronize the Core-owned runtime lifecycle so enabled plugins are loaded
and disabled plugins are unloaded through the same path.

Current external routes include device status, wave control, script run,
script document management, and plugin registry management. See
`docs/external-control/websocket.md`.

## Script Flow

Script documents are stored in SQLite, validated by `crates/script`, and run
through a storage-backed Core runner. The runner compiles the document, enqueues
the compiled script, and a background worker executes steps through an injected
Core action executor. This keeps IPC responsive and gives future device/plugin
actions a replaceable boundary. Plugin hook script steps invoke enabled plugins
through the same sandboxed runtime host used by UI and WebSocket plugin
management.
