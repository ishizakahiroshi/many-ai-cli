//go:build !windows

package wrapper

// answerTerminalQueriesLocally enables DSR/DA replies in ptyPump on Unix.
// creack/pty is not an emulator; VTE/ConPTY answer these queries locally, so
// Claude Code's Ink would otherwise hang in raw-mode standby on Linux Hub.
const answerTerminalQueriesLocally = true
