# Plugin Runtime ABI

ArcFlow plugins run in sandboxed WASM or JavaScript runtimes. The runtime engine
boundary is JSON-based so both engines share one host contract.

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

An empty output is explicit:

```json
{
  "actions": []
}
```
