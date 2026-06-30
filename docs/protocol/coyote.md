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
- Coyote V2 uses three-byte little-endian bit fields for AB strength and waveform frames.
- Coyote V3 uses B0 write commands, B1 strength notifications, and BF soft-limit/balance commands.
- Invalid V3 channel wave values are preserved during parsing because the protocol documents them as a way to make the device ignore a channel.

## Safety notes

Protocol construction is not a complete safety policy. Higher-level Rust crates must still apply user limits, soft-limit setup, rate limits, audit logging, emergency stop handling, and external-control permission checks before writing frames to a device.
