# Plugin Runtime ABI

ArcFlow plugins run in sandboxed WASM or JavaScript runtimes. The runtime engine
boundary is JSON-based so both engines share one host contract.

When a plugin is installed from disk, Rust Core gives the runtime engine a load
request containing the manifest and bundle root. The engine resolves the entry
file from that context, but plugin code only receives invocation envelopes.
The current validation adapters check bundle-backed entries before recording
lifecycle state: WASM entries must parse as WebAssembly modules, and JavaScript
entries must be non-empty UTF-8 module source. Manifest-only plugins still use
recording lifecycle for development.

Bundle-backed WASM and JavaScript plugins may declare hook outputs with the
same `arcflowPlugin` JSON object. This keeps early plugins sandboxed and
deterministic while real engine call conventions are attached behind the same
runtime boundary.

For WASM, embed the JSON object in a custom section named `arcflowPlugin`.

For JavaScript, export the JSON object:

```js
export const arcflowPlugin = {
  "hooks": {
    "device.connected": {
      "actions": [
        {
          "method": "storage.private.put",
          "params": {
            "key": "lastDevice",
            "value": {
              "deviceId": "coyote-v3"
            }
          }
        }
      ]
    }
  }
};
```

Hook names in `hooks` map to the same `PluginOutput` envelope described below.
Hooks not listed in the object return an explicit empty output.

## Invocation

ArcFlow invokes a loaded plugin with a `PluginInvocation` envelope:

```json
{
  "hook": "script.afterStep",
  "payload": {
    "step": 3
  }
}
```

`hook` is the host event or method being invoked. `payload` is arbitrary JSON
owned by the host feature that emitted the hook.

## Output

Plugins return a `PluginOutput` envelope:

```json
{
  "actions": [
    {
      "method": "storage.private.put",
      "params": {
        "key": "settings",
        "value": {
          "enabled": true
        }
      }
    }
  ]
}
```

Each action is routed through Rust Core's Plugin API. The API checks the
plugin's declared manifest capabilities before it performs any host operation.
Plugins never call Bluetooth, SQLite, files, or platform APIs directly.
Device actions may be asynchronous because they go through Core-owned discovery
and BLE state.

Current host actions:

| Method | Required capability | Purpose |
| --- | --- | --- |
| `device.scan` | `device.read` | Refresh Core-owned device discovery and return the adapter status plus discovered devices. |
| `device.status` | `device.read` | Read one device's status through Core-owned device discovery. |
| `device.activateOutput` | `wave.control` | Mark a device as active for Core-owned output writes. |
| `device.deactivateOutput` | `wave.control` | Remove a device from active Core-owned output writes. |
| `storage.private.put` | `storage.private` | Store plugin-private JSON. |
| `storage.private.get` | `storage.private` | Read plugin-private JSON. |
| `storage.private.delete` | `storage.private` | Delete one plugin-private key. |
| `storage.private.keys` | `storage.private` | List plugin-private keys. |
| `wave.submitWindow` | `wave.control` | Submit one validated Coyote V3 output window through Core-owned output control. |
| `wave.stop` | `wave.control` | Stop Core-owned output through the attached device output controller. |

An empty output is explicit:

```json
{
  "actions": []
}
```

`device.scan` returns the Core-owned discovery shape. Platform BLE diagnostics
stay on the Tauri IPC and local plugin bridge response because they are adapter
metadata rather than plugin-visible device state:

```json
{
  "adapterStatus": "ready",
  "devices": [
    {
      "deviceId": "coyote-v3",
      "model": "coyoteV3",
      "connected": true,
      "batteryPercent": 87
    }
  ]
}
```
