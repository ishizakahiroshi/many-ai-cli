# Synthetic terminal oracle

Source: `internal/hub/vt_buffer.go` at fixed Go revision
`21d0bc7935a2c4696fb89ccff2e324157a528c2d`.

`generate_oracle.py` reads that immutable Git object, copies it into an isolated
standalone temporary Go package, and feeds 200 deterministic synthetic sequences
(seed 214). It does not change Go source, start a Hub, or invoke a provider.
Caller selects the Go compiler and synthetic work/cache roots explicitly.

The committed JSON contains whole-write Go snapshots, cursor and mode state.
`terminal_contracts::go_oracle_synthetic_snapshots_and_all_byte_partitions` also
checks Rust for each two-way byte split and chunk widths 1–17.

Intentional strengthening: OSC/DCS/SOS/PM/APC ST (`ESC \\`) termination now survives
an arbitrary chunk boundary. Go only recognizes the two bytes within one write;
the Rust expected result is the Go whole-write snapshot. A separate test covers
all five string introducers, BEL/ST endings and every boundary. This is explicitly
not claimed to reproduce the Go split-ST bug.

Scope: pure VT/replay/input state only. Actual application session integration,
frontend display and native PTY/process teardown are separate acceptance gates.
