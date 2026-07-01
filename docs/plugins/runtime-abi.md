# Plugin Runtime ABI

ArcFlow plugins run in sandboxed WASM or JavaScript runtimes. The runtime engine
boundary is JSON-based so both engines share one host contract.

When a plugin is installed from disk, Rust Core gives the runtime engine a load
request containing the manifest and bundle root. The engine resolves the entry
file from that context, but plugin code only receives invocation envelopes.
The current validation adapters check bundle-backed entries before recording
lifecycle state: WASM entries must parse as WebAssembly modules, and JavaScript
entries must be non-empty UTF-8 module source. Manifest-only plugins still use
recording lifecycle for development; the execution call convention is still
being wired behind the same ABI.

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

Current host actions:

| Method | Required capability | Purpose |
| --- | --- | --- |
| `storage.private.put` | `storage.private` | Store plugin-private JSON. |
| `storage.private.get` | `storage.private` | Read plugin-private JSON. |
| `storage.private.delete` | `storage.private` | Delete one plugin-private key. |
| `storage.private.keys` | `storage.private` | List plugin-private keys. |
| `wave.stop` | `wave.control` | Stop Core-owned output through the attached device output controller. |

An empty output is explicit:

```json
{
  "actions": []
}
```
