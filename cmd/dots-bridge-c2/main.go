// dots-bridge-c2 is an isolated, offline-only bridge prototype. It does not
// connect to a Hub, open a transport, or start or stop an AI process.
package main

import (
	"encoding/json"
	"errors"
	"flag"
	"io"
	"os"
	"strings"
	"time"

	"many-ai-cli/internal/dotsbridge"
)

const (
	exitOK      = 0
	exitFailure = 1
	exitUsage   = 2
)

func main() {
	os.Exit(run(os.Args[1:], os.Stdout, os.Stderr))
}

// run keeps CLI tests in-process: no live transport or subprocess is needed.
func run(args []string, stdout, stderr io.Writer) int {
	// Reject live requests before flag parsing or any database access. Even
	// --live=false is unsupported: no live mode exists in this executable.
	for _, arg := range args {
		if arg == "--live" || strings.HasPrefix(arg, "--live=") || arg == "-live" || strings.HasPrefix(arg, "-live=") {
			return fail(stderr, exitUsage, "offline_only", "live operation is unavailable; this executable supports offline mocks only")
		}
	}
	if len(args) == 0 {
		return usage(stdout, stderr)
	}
	switch args[0] {
	case "help", "--help", "-h":
		if len(args) != 1 {
			return fail(stderr, exitUsage, "invalid_arguments", "help does not accept additional arguments")
		}
		return usage(stdout, stderr)
	case "live", "relay", "local":
		return fail(stderr, exitUsage, "offline_only", "live operation is unavailable; this executable supports offline mocks only")
	case "trial", "status", "stop", "export":
		return runCommand(args[0], args[1:], stdout, stderr)
	default:
		return fail(stderr, exitUsage, "unknown_command", "unknown command; use help to list offline commands")
	}
}

func runCommand(command string, args []string, stdout, stderr io.Writer) int {
	flags := flag.NewFlagSet(command, flag.ContinueOnError)
	// flag's diagnostics echo arbitrary input. Return fixed, bounded errors
	// instead, including for an unbounded or control-character argument.
	flags.SetOutput(io.Discard)
	dbPath := flags.String("db", "", "dedicated bridge SQLite database path")
	var mock bool
	var campaign, route, caseID string
	if command == "trial" {
		flags.BoolVar(&mock, "mock", false, "run offline fake adapters only")
		flags.StringVar(&campaign, "campaign", "", "campaign identifier")
		flags.StringVar(&route, "path", "mcp", "mock route: mcp or socket")
		flags.StringVar(&caseID, "case", "color", "mock case: color or recovery")
	}
	if err := flags.Parse(args); err != nil {
		if errors.Is(err, flag.ErrHelp) {
			return usage(stdout, stderr)
		}
		return fail(stderr, exitUsage, "invalid_arguments", "invalid command arguments; use help for supported flags")
	}
	if flags.NArg() != 0 {
		return fail(stderr, exitUsage, "invalid_arguments", "unexpected positional arguments; use help for supported flags")
	}
	if strings.TrimSpace(*dbPath) == "" {
		return fail(stderr, exitUsage, "database_required", "an explicit --db path is required")
	}
	if command == "trial" {
		if !mock {
			return fail(stderr, exitUsage, "mock_required", "trial requires explicit --mock; live operation is unavailable")
		}
		if strings.TrimSpace(campaign) == "" {
			return fail(stderr, exitUsage, "campaign_required", "trial requires a nonempty --campaign identifier")
		}
		if route != string(dotsbridge.RouteM) && route != string(dotsbridge.RouteS) {
			return fail(stderr, exitUsage, "invalid_path", "--path must be mcp or socket")
		}
		if caseID != "color" && caseID != "recovery" {
			return fail(stderr, exitUsage, "invalid_case", "--case must be color or recovery")
		}
	} else {
		// Read/stop commands must not initialize an empty queue when a path is
		// mistyped. Open performs the database's own identity validation.
		info, err := os.Stat(*dbPath) // #nosec G703 -- Explicit operator-selected local ledger, never an event path; Open verifies exclusive bridge ownership.
		if err != nil {
			return fail(stderr, exitFailure, "database_unavailable", "the existing bridge database is unavailable")
		}
		if !info.Mode().IsRegular() {
			return fail(stderr, exitFailure, "invalid_database", "--db must identify an existing regular bridge database file")
		}
		if info.Size() == 0 {
			return fail(stderr, exitFailure, "database_unavailable", "the existing bridge database is unavailable")
		}
	}

	store, err := dotsbridge.Open(*dbPath)
	if err != nil {
		return fail(stderr, exitFailure, "database_open_failed", "unable to open the dedicated bridge database")
	}
	result, err := execute(command, store, campaign, dotsbridge.Route(route), caseID)
	closeErr := store.Close()
	if err != nil {
		return fail(stderr, exitFailure, "command_failed", "offline command failed; inspect the bridge status and export for reconciliation")
	}
	if closeErr != nil {
		return fail(stderr, exitFailure, "database_close_failed", "unable to close the bridge database cleanly; inspect status before retrying")
	}
	return emit(stdout, stderr, result)
}

func execute(command string, store *dotsbridge.Store, campaign string, route dotsbridge.Route, caseID string) (any, error) {
	switch command {
	case "trial":
		report, err := dotsbridge.RunMock(store, campaign, route, caseID, time.Now().UTC())
		return struct {
			Offline bool                   `json:"offline"`
			Report  dotsbridge.TrialReport `json:"report"`
		}{true, report}, err
	case "status":
		jobs, err := store.Status()
		if jobs == nil {
			jobs = []dotsbridge.Job{}
		}
		return struct {
			Offline bool             `json:"offline"`
			Jobs    []dotsbridge.Job `json:"jobs"`
		}{true, jobs}, err
	case "export":
		records, err := store.Export()
		return struct {
			Offline bool              `json:"offline"`
			Records dotsbridge.Export `json:"records"`
		}{true, records}, err
	case "stop":
		if err := store.Stop(time.Now().UTC()); err != nil {
			return nil, err
		}
		return struct {
			Offline           bool   `json:"offline"`
			Stopped           bool   `json:"stopped"`
			Scope             string `json:"scope"`
			AIProcessesKilled bool   `json:"ai_processes_killed"`
		}{true, true, "bridge_only", false}, nil
	default:
		return nil, errors.New("unsupported offline command")
	}
}

func usage(stdout, stderr io.Writer) int {
	return emit(stdout, stderr, struct {
		Name         string   `json:"name"`
		Offline      bool     `json:"offline"`
		Commands     []string `json:"commands"`
		Restrictions []string `json:"restrictions"`
	}{
		Name:    "dots-bridge-c2",
		Offline: true,
		Commands: []string{
			"help",
			"trial --mock --db <dedicated.db> --campaign <id> [--path mcp|socket] [--case color|recovery]",
			"status --db <existing.db>",
			"stop --db <existing.db>",
			"export --db <existing.db>",
		},
		Restrictions: []string{
			"Offline fake adapters only; no network, public relay, or AI process execution.",
			"Trial defaults: --path mcp --case color. Campaign budgets persist in the dedicated database.",
			"Stop disables bridge work only; it never kills AI processes.",
			"Mock results do not prove real MCP/Socket protocol, dots continuation, or live UX.",
		},
	})
}

func emit(stdout, stderr io.Writer, result any) int {
	if err := json.NewEncoder(stdout).Encode(result); err != nil {
		return fail(stderr, exitFailure, "output_failed", "unable to write JSON output; inspect status before retrying")
	}
	return exitOK
}

func fail(stderr io.Writer, code int, name, message string) int {
	// All messages are fixed literals, never database content or raw input.
	_ = json.NewEncoder(stderr).Encode(struct {
		Error   string `json:"error"`
		Message string `json:"message"`
	}{name, message})
	return code
}
