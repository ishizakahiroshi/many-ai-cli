package wrapper

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"unicode"
	"unicode/utf16"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/headless"
	"many-ai-cli/internal/sessionlog"
)

// launch_prompt.go hands an interactive child its first instruction as the
// CLI's first launch argument, instead of the Hub typing it into the screen
// (子 plan: docs/local/plan_child-launch-prompt-and-trust_c4_launch-arg-prompt.md 内部 C2).
//
// Why an argument. Typing into a screen the Hub cannot read is what went wrong
// on 2026-09-24 (#53): the Enter meant to submit the instruction answered
// Claude Code's folder-trust dialog, whose default was "No, exit". A CLI that
// takes the instruction as an argument holds it through its own startup
// dialogs and runs it afterwards. Which CLIs do is data in
// internal/config/launch_prompt.go, measured per provider.
//
// Why first. The argument list already carries a variadic flag (claude's
// `--allowedTools a b …` at the bounded tier) and flags the wrapper appends
// later (`--settings`, `--append-system-prompt-file`). A positional placed
// after a variadic flag is swallowed as one of its values. headless.BuildArgv
// puts its prompt first for the same reason, and the measured launches (the
// parent plan's T1/T2) had the instruction before `--settings`.
//
// Why sometimes a file. On Windows, a CLI installed as an npm shim that the
// wrapper cannot unwrap to a real .exe is started as `cmd.exe /c <shim> …`,
// and cmd.exe cuts an argument at its first newline (measured with codex:
// only the first of two lines arrived). A very long instruction also runs into
// the Windows command-line limit (32,767 UTF-16 units for the whole line). In
// both cases the instruction goes into a private file and the argument is a
// single line pointing at it.
//
// What stays visible. The argument — the instruction itself, or the pointer —
// can be read from the machine's process list while the CLI runs. The five
// headless providers that take their prompt as an argument are already in the
// same position, and the Hub-to-wrapper hand-over stays a file
// (--prompt-file), so the instruction never sits in the wrapper's own
// command line or in any log. The instruction text is never logged here.

// launchPromptMaxArgCost bounds an instruction passed as one argument, in
// Windows command-line units (see launchPromptArgCost). It is the 32,767-unit
// limit for the whole line minus room for the executable path and the
// arguments the wrapper adds; a planning estimate, not a measured threshold.
const launchPromptMaxArgCost = 24000

// launchPromptPointerFormat is the one line passed instead of the instruction
// when it goes into a file. One line and plain words: it has to survive
// cmd.exe, and the model has to act on it without further context.
const launchPromptPointerFormat = "Your instructions are in the file %s. Read that file now and follow it exactly."

// launchPromptShellUnsafe are the characters cmd.exe may still interpret
// inside a quoted argument (variable expansion, escapes, redirection), so a
// pointer whose path carries one cannot be trusted to arrive unchanged.
const launchPromptShellUnsafe = `%!^&|<>"`

// launchPromptFilePrefix names the pointer files. The wrapper's PID is part of
// the name so the Hub's collection (internal/hub: sweepStaleHeadlessPrompts)
// can tell a file whose wrapper is still running — and whose CLI may not have
// read it yet, because it is still waiting on a trust dialog — from one a
// killed wrapper left behind.
const launchPromptFilePrefix = "launch-"

// LaunchPromptArgUsable reports whether this machine can hand provider its
// first instruction as a launch argument, and why not when it cannot. The Hub
// asks before choosing the route, because a launch that fails here cannot fall
// back on its own: typing is the Hub's job, not the wrapper's.
//
// The one case refused today: the launch goes through cmd.exe and either the
// shim's path contains whitespace (cmd.exe would then strip the quotes around
// the executable as soon as a second quoted argument appears — the pointer
// line is one) or the pointer directory's path carries a character cmd.exe
// interprets. Everything else can pass the instruction directly or through a
// pointer.
func LaunchPromptArgUsable(provider string) (bool, string) {
	dir, err := launchPromptDir()
	if err != nil {
		return false, "cannot resolve the many-ai-cli temp directory"
	}
	return launchPromptArgUsable(provider, nil, dir)
}

func launchPromptArgUsable(provider string, customArgv []string, dir string) (bool, string) {
	shim, throughShell := launchShellShim(provider, customArgv)
	if !throughShell {
		return true, ""
	}
	if strings.ContainsAny(shim, " \t") {
		return false, "the launch goes through cmd.exe and the CLI's shim path contains whitespace"
	}
	if strings.ContainsAny(dir, launchPromptShellUnsafe) {
		return false, "the launch goes through cmd.exe and the temp directory path contains a character cmd.exe interprets"
	}
	return true, ""
}

// launchPromptDir is where pointer files go: the same private directory the
// Hub uses for the --prompt-file hand-over.
func launchPromptDir() (string, error) {
	dir, err := config.Dir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "tmp"), nil
}

// preparedLaunchPrompt is the first argument an interactive launch starts the
// CLI with.
type preparedLaunchPrompt struct {
	// Arg is the instruction itself or the one-line pointer. Empty when there
	// is nothing to pass.
	Arg string
	// Pointer is true when Arg points at a file instead of carrying the
	// instruction.
	Pointer bool
	// Path is the pointer file, when one was written.
	Path string
}

// prepareLaunchPrompt consumes the Hub's --prompt-file, fills in the session
// id the wrapper just registered with, and decides how the instruction reaches
// the CLI. The returned cleanup removes the pointer file; the caller defers it
// so the file lives exactly as long as the session.
func prepareLaunchPrompt(promptPath string, sessionID int, provider string, customArgv []string, dir string, pid int) (preparedLaunchPrompt, func(), error) {
	prompt, err := takePromptFile(promptPath)
	if err != nil {
		return preparedLaunchPrompt{}, nil, err
	}
	// The Hub wrote the instruction before this session had an id
	// (internal/headless/prompt.go explains the placeholder).
	prompt = headless.ExpandPrompt(prompt, sessionID)
	if strings.TrimSpace(prompt) == "" {
		return preparedLaunchPrompt{}, nil, nil
	}
	_, throughShell := launchShellShim(provider, customArgv)
	if !throughShell && launchPromptArgCost(prompt) <= launchPromptMaxArgCost {
		return preparedLaunchPrompt{Arg: launchPromptArg(prompt)}, nil, nil
	}
	if ok, reason := launchPromptArgUsable(provider, customArgv, dir); !ok {
		// The Hub checks the same thing before choosing this route, so this is
		// only reached by a `wrap --prompt-file` typed by hand.
		return preparedLaunchPrompt{}, nil, fmt.Errorf("cannot pass the instruction at launch: %s", reason)
	}
	path, err := writeLaunchPromptFile(dir, pid, prompt)
	if err != nil {
		return preparedLaunchPrompt{}, nil, err
	}
	cleanup := func() { _ = os.Remove(path) }
	return preparedLaunchPrompt{Arg: fmt.Sprintf(launchPromptPointerFormat, path), Pointer: true, Path: path}, cleanup, nil
}

// withLaunchPrompt returns the argument list the CLI starts with: the
// instruction (or pointer) first, then everything else unchanged. The caller
// passes the list only after every later addition (`--settings`,
// `--append-system-prompt-file`) is already in it, so nothing lands in front of
// the instruction or between a variadic flag and its values.
func withLaunchPrompt(providerArgs []string, arg string) []string {
	out := make([]string, 0, len(providerArgs)+1)
	out = append(out, arg)
	return append(out, providerArgs...)
}

// launchPromptArg keeps an instruction that happens to start with "-" from
// being read as a flag: a leading space is harmless to the model and makes the
// argument a positional to every flag parser.
//
// The same space keeps a one-word instruction from being read as a
// subcommand. The instruction is the CLI's first argument, and a first
// argument equal to a subcommand name (claude's update / doctor / mcp, codex's
// resume / login / exec, ...) starts that command instead of a session. Only a
// word without whitespace can equal a name, so longer instructions pass as they
// are.
func launchPromptArg(prompt string) string {
	if strings.HasPrefix(prompt, "-") || !strings.ContainsFunc(prompt, unicode.IsSpace) {
		return " " + prompt
	}
	return prompt
}

// launchPromptArgCost is an upper bound on what prompt occupies in a Windows
// command line: its UTF-16 length, plus one for every character the argument
// quoting may escape (a `"` becomes `\"`, backslashes before one double), plus
// the surrounding quotes. It is also a safe bound elsewhere: 24,000 UTF-16
// units are at most 72,000 bytes of UTF-8, well under Linux's 128 KiB limit
// for a single argument.
func launchPromptArgCost(prompt string) int {
	return len(utf16.Encode([]rune(prompt))) + strings.Count(prompt, `"`) + strings.Count(prompt, `\`) + 2
}

// LaunchPromptFileOwner reports whether name is a pointer file this package
// writes and, if so, the PID of the wrapper that wrote it. The Hub's
// collection of abandoned files uses it to leave alone a file whose wrapper is
// still running: that CLI may still be waiting on a trust dialog and not have
// read its instructions yet, however long ago the file was written.
func LaunchPromptFileOwner(name string) (int, bool) {
	if !strings.HasPrefix(name, launchPromptFilePrefix) || !strings.HasSuffix(name, ".md") {
		return 0, false
	}
	rest := strings.TrimPrefix(name, launchPromptFilePrefix)
	pidText, _, found := strings.Cut(rest, "-")
	if !found {
		return 0, false
	}
	pid, err := strconv.Atoi(pidText)
	if err != nil || pid <= 0 {
		return 0, false
	}
	return pid, true
}

func writeLaunchPromptFile(dir string, pid int, prompt string) (string, error) {
	if err := os.MkdirAll(dir, sessionlog.PrivateDirMode); err != nil {
		return "", fmt.Errorf("launch prompt dir: %w", err)
	}
	var raw [16]byte
	if _, err := rand.Read(raw[:]); err != nil {
		return "", fmt.Errorf("launch prompt name: %w", err)
	}
	path := filepath.Join(dir, fmt.Sprintf("%s%d-%s.md", launchPromptFilePrefix, pid, hex.EncodeToString(raw[:])))
	if err := os.WriteFile(path, []byte(prompt), sessionlog.PrivateFileMode); err != nil {
		return "", fmt.Errorf("launch prompt file: %w", err)
	}
	return path, nil
}
