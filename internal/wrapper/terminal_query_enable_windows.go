//go:build windows

package wrapper

// answerTerminalQueriesLocally is false on Windows: ConPTY already answers
// DSR/DA inside the host console stack. Answering again in the wrap would
// risk double CPR/DA replies.
const answerTerminalQueriesLocally = false
