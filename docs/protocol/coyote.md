# Coyote protocol scope

ArcFlow currently implements protocol-byte parsing and construction for the DG-LAB Coyote pulse host only.

Source material:

- https://github.com/dungeonlab-open/dglab-bluetooth-protocol
- https://github.com/dungeonlab-open/dglab-bluetooth-protocol/blob/main/coyote/README.md
- https://github.com/dungeonlab-open/dglab-bluetooth-protocol/blob/main/coyote/v2/README.md
- https://github.com/dungeonlab-open/dglab-bluetooth-protocol/blob/main/coyote/v3/README.md

## Implementation notes

- Bluetooth connection management stays out of `arcflow-protocol`.
- `crates/protocol` owns byte-level protocol frames, ranges, and conversion helpers.
- `crates/core` owns BLE service/characteristic metadata and expands 16-bit UUIDs
  into canonical Bluetooth base UUID strings for platform adapters.
- The `0x180A` / `0x1500` battery characteristic is parsed as a shared Coyote
  one-byte `0..=100` percentage instead of being tied to one protocol version.
- Coyote V2 uses three-byte little-endian bit fields for AB strength and waveform frames.
- Coyote V3 uses B0 write commands, B1 strength notifications, and BF soft-limit/balance commands.
- A connected Coyote V3 session subscribes to both `0x150B` B1 notifications and
  the shared `0x1500` battery characteristic for status updates.
- Rust Core builds BF writes from `SafetyLimits` and sends them through the same
  `0x150A` write characteristic before a Coyote V3 device is activated for output.
- Invalid V3 channel wave values are preserved during parsing because the protocol documents them as a way to make the device ignore a channel.

## BLE UUIDs

| Purpose | Service | Characteristic |
| --- | --- | --- |
| Battery | `0x180A` (`0000180a-0000-1000-8000-00805f9b34fb`) | `0x1500` (`00001500-0000-1000-8000-00805f9b34fb`) |
| Coyote V2 AB strength | `0x180B` (`0000180b-0000-1000-8000-00805f9b34fb`) | `0x1504` (`00001504-0000-1000-8000-00805f9b34fb`) |
| Coyote V2 A waveform | `0x180B` (`0000180b-0000-1000-8000-00805f9b34fb`) | `0x1505` (`00001505-0000-1000-8000-00805f9b34fb`) |
| Coyote V2 B waveform | `0x180B` (`0000180b-0000-1000-8000-00805f9b34fb`) | `0x1506` (`00001506-0000-1000-8000-00805f9b34fb`) |
| Coyote V3 write | `0x180C` (`0000180c-0000-1000-8000-00805f9b34fb`) | `0x150A` (`0000150a-0000-1000-8000-00805f9b34fb`) |
| Coyote V3 notify | `0x180C` (`0000180c-0000-1000-8000-00805f9b34fb`) | `0x150B` (`0000150b-0000-1000-8000-00805f9b34fb`) |

## Safety notes

Protocol construction is not a complete safety policy. Higher-level Rust crates must still apply user limits, soft-limit setup, rate limits, audit logging, emergency stop handling, and external-control permission checks before writing frames to a device.
