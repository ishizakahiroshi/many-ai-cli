# Native Windows spawn and ConPTY standard handles

> 最終更新: 2026-10-06(火) 01:47:03 UTC

## Evidence and correction

The owner reproduced an immediate `set /p` failure at public baseline `4dc961ce600d12aa0911e5a9620e50411ab13d9f` with an isolated trial Hub, synthetic home/port, and copied `cmd.exe` provider. The owner's independent controls distinguished working ConPTY input from redirected stdin at EOF. Those observations are owner evidence, not a native run in this Linux workspace.

The source discrepancy is independently confirmed: `rust/src/process/pty_windows.rs` creates `STARTUPINFOEXW` with zero flags; the fixed Go oracle `21d0bc7935a2c4696fb89ccff2e324157a528c2d` pins go-pty v0.2.3, whose `cmd_windows.go` sets `STARTF_USESTDHANDLES` on zeroed standard handles. The actual Rust wrapper parent has NUL stdin and redirected spawn-log stdout/stderr.

[Microsoft Terminal discussion 15814](https://github.com/microsoft/terminal/discussions/15814) describes redirected parent standard handles reaching a ConPTY child despite disabled handle inheritance, and recommends `STARTF_USESTDHANDLES` with null standard handles. [GetStdHandle documentation](https://learn.microsoft.com/en-us/windows/console/getstdhandle) explains that null entries are replaced by console handles when attaching. This supports the mechanism and focused correction; final application behavior still requires the native runner.

The correction explicitly sets the flag and null stdin/stdout/stderr. It retains the HPCON attribute, `bInheritHandles=FALSE`, suspended creation, shared kill-on-close Job assignment before resume, cancellation, pipe servicing and reaping. The completed 1–10 work remains a separate local checkpoint `d4695c87b0190468d8783f3edcc4dc300395e835`.

## Native acceptance boundary

The [native integration regression](../../../rust/tests/native_windows_spawn.rs) uses the real built Hub, its HTTP spawn route, actual wrapper and copied `cmd.exe` provider in a synthetic trial root. It requires a live wrapper and an explicit standby/idle-activity snapshot after a three-second no-input interval, stdout and stderr markers over PTY, matching `hello` input/output, then UI completion and persisted provider exit zero after `quit`. It also checks wrapper exit and authenticated Hub shutdown/reaping. Failure cleanup closes and reaps only the owned fixture Job before propagating the assertion. No PTY/wrapper mock, ignored test or timeout-as-success is acceptable.

The current wire contract requires two independent observations: the UI receives `session_end` with completed state, while the wrapper's actual exit code is recorded in the session JSONL. The UI message omits `exit_code`; an absent JSON field or a deserializer's default zero is not itself proof of provider success. The regression must check the persisted wrapper exit code as zero as well as the UI terminal state.

Rust 1.90 formatting passed. A temporary Linux host-check copy of the same test body, with only its Windows crate gate removed, passed `cargo test --no-run` and strict Clippy. That copy was never executed and was removed; the committed target retains `#![cfg(windows)]`. This checks shared API/type correctness without claiming native ConPTY behavior.

At64663728 the Windows test compiled and ran, but stopped on its incorrect initial running-state expectation before checking PTY markers or input. The A2 successor corrects initial standby and requires positive idle-activity evidence after the no-input interval; the ConPTY effect remains unverified until that successor actually completes the native round-trip. Final exact-SHA four-target CI/artifacts remain pending. The earlier Linux 1,394-test result covers the separately recorded 1–10 source; it does not validate ConPTY. AuthenticationExpired on wrong-token sockets and ordinary disconnect warning policy are outside this correction.
