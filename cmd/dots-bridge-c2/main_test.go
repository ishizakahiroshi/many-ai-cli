package main

import (
	"bytes"
	"database/sql"
	"encoding/json"
	"errors"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func invoke(t *testing.T, args ...string) (int, map[string]json.RawMessage, map[string]string) {
	t.Helper()
	var stdout, stderr bytes.Buffer
	code := run(args, &stdout, &stderr)
	var output map[string]json.RawMessage
	var diagnostic map[string]string
	if stdout.Len() != 0 {
		if err := json.Unmarshal(stdout.Bytes(), &output); err != nil {
			t.Fatalf("invalid stdout JSON: %v; output=%q", err, stdout.String())
		}
	}
	if stderr.Len() != 0 {
		if err := json.Unmarshal(stderr.Bytes(), &diagnostic); err != nil {
			t.Fatalf("invalid stderr JSON: %v; output=%q", err, stderr.String())
		}
		if stderr.Len() > 512 {
			t.Fatalf("diagnostic exceeds bound: %d bytes", stderr.Len())
		}
	}
	if code == exitOK && stderr.Len() != 0 {
		t.Fatalf("successful command wrote stderr: %q", stderr.String())
	}
	if code != exitOK && stdout.Len() != 0 {
		t.Fatalf("failed command wrote stdout: %q", stdout.String())
	}
	return code, output, diagnostic
}

func assertMissing(t *testing.T, path string) {
	t.Helper()
	for _, suffix := range []string{"", "-wal", "-shm", "-journal"} {
		if _, err := os.Stat(path + suffix); !os.IsNotExist(err) {
			t.Fatalf("unexpected database artifact %q: %v", path+suffix, err)
		}
	}
}

func TestHelpIsOfflineJSON(t *testing.T) {
	for _, args := range [][]string{nil, {"help"}, {"--help"}, {"-h"}, {"trial", "--help"}, {"status", "-h"}} {
		t.Run(strings.Join(args, "_"), func(t *testing.T) {
			code, out, _ := invoke(t, args...)
			if code != exitOK || string(out["offline"]) != "true" || string(out["name"]) != `"dots-bridge-c2"` {
				t.Fatalf("unexpected help response: code=%d output=%v", code, out)
			}
			var commands []string
			if err := json.Unmarshal(out["commands"], &commands); err != nil || len(commands) != 5 {
				t.Fatalf("missing command help: commands=%v err=%v", commands, err)
			}
		})
	}
}

func TestRejectLiveBeforeOpeningDatabase(t *testing.T) {
	for _, prefix := range [][]string{
		{"live"}, {"relay"}, {"local"}, {"trial", "--live"},
		{"trial", "--mock", "--live=true"}, {"trial", "--mock", "--live=false"},
		{"trial", "--mock", "-live"}, {"trial", "--mock", "-live=false"},
		{"status", "--live"}, {"help", "--live"},
	} {
		t.Run(strings.Join(prefix, "_"), func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "must-not-exist.db")
			args := append(append([]string{}, prefix...), "--db", path, "--campaign", "offline-test")
			code, _, diagnostic := invoke(t, args...)
			if code != exitUsage || diagnostic["error"] != "offline_only" {
				t.Fatalf("live request not rejected: code=%d diagnostic=%v", code, diagnostic)
			}
			assertMissing(t, path)
		})
	}
}

func TestInvalidArgumentsDoNotCreateDatabase(t *testing.T) {
	tests := []struct {
		name string
		args []string
		want string
	}{
		{"missing_mock", []string{"trial", "--campaign", "test"}, "mock_required"},
		{"false_mock", []string{"trial", "--mock=false", "--campaign", "test"}, "mock_required"},
		{"missing_campaign", []string{"trial", "--mock"}, "campaign_required"},
		{"blank_campaign", []string{"trial", "--mock", "--campaign", " \n\t"}, "campaign_required"},
		{"invalid_route", []string{"trial", "--mock", "--campaign", "test", "--path", "http"}, "invalid_path"},
		{"invalid_case", []string{"trial", "--mock", "--campaign", "test", "--case", "live"}, "invalid_case"},
		{"unknown_flag", []string{"trial", "--mock", "--campaign", "test", "--token", "secret"}, "invalid_arguments"},
		{"missing_value", []string{"trial", "--mock", "--campaign", "test", "--case"}, "invalid_arguments"},
		{"bad_bool", []string{"trial", "--mock=perhaps", "--campaign", "test"}, "invalid_arguments"},
		{"positional", []string{"trial", "--mock", "--campaign", "test", "unexpected"}, "invalid_arguments"},
		{"status_rejects_mock", []string{"status", "--mock"}, "invalid_arguments"},
		{"unknown_command", []string{"serve"}, "unknown_command"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "untouched.db")
			// Put --db before test flags so a missing final flag value remains missing.
			args := append([]string{tt.args[0], "--db", path}, tt.args[1:]...)
			code, _, diagnostic := invoke(t, args...)
			if code != exitUsage || diagnostic["error"] != tt.want {
				t.Fatalf("code=%d diagnostic=%v; want %s", code, diagnostic, tt.want)
			}
			assertMissing(t, path)
		})
	}
}

func TestExplicitDatabaseRequired(t *testing.T) {
	for _, command := range []string{"trial", "status", "stop", "export"} {
		t.Run(command, func(t *testing.T) {
			code, _, diagnostic := invoke(t, command)
			if code != exitUsage || diagnostic["error"] != "database_required" {
				t.Fatalf("code=%d diagnostic=%v", code, diagnostic)
			}
		})
	}
}

func TestExistingDatabaseCommandsDoNotCreateMissingFile(t *testing.T) {
	for _, command := range []string{"status", "stop", "export"} {
		t.Run(command, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "missing.db")
			code, _, diagnostic := invoke(t, command, "--db", path)
			if code != exitFailure || diagnostic["error"] != "database_unavailable" {
				t.Fatalf("code=%d diagnostic=%v", code, diagnostic)
			}
			assertMissing(t, path)
		})
	}
}

func TestExistingDatabaseCommandsRejectDirectory(t *testing.T) {
	for _, command := range []string{"status", "stop", "export"} {
		t.Run(command, func(t *testing.T) {
			path := t.TempDir()
			code, _, diagnostic := invoke(t, command, "--db", path)
			if code != exitFailure || diagnostic["error"] != "invalid_database" {
				t.Fatalf("code=%d diagnostic=%v", code, diagnostic)
			}
			entries, err := os.ReadDir(path)
			if err != nil || len(entries) != 0 {
				t.Fatalf("directory was changed: entries=%v err=%v", entries, err)
			}
		})
	}
}

func TestExistingDatabaseCommandsDoNotInitializeEmptyFile(t *testing.T) {
	for _, command := range []string{"status", "stop", "export"} {
		t.Run(command, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "empty.db")
			if err := os.WriteFile(path, nil, 0600); err != nil {
				t.Fatal(err)
			}
			code, _, diagnostic := invoke(t, command, "--db", path)
			if code != exitFailure || diagnostic["error"] != "database_unavailable" {
				t.Fatalf("code=%d diagnostic=%v", code, diagnostic)
			}
			info, err := os.Stat(path)
			if err != nil || info.Size() != 0 {
				t.Fatalf("empty file was initialized: info=%v err=%v", info, err)
			}
		})
	}
}

func TestCommandsDoNotChangeUnrelatedSQLiteDatabase(t *testing.T) {
	for _, table := range []string{"unrelated_data", "jobs"} {
		for _, command := range []string{"trial", "status", "stop", "export"} {
			t.Run(table+"_"+command, func(t *testing.T) {
				path := filepath.Join(t.TempDir(), "unrelated.db")
				db, err := sql.Open("sqlite", path)
				if err != nil {
					t.Fatal(err)
				}
				if _, err = db.Exec("CREATE TABLE " + table + " (value TEXT); INSERT INTO " + table + " VALUES ('keep me')"); err != nil {
					db.Close()
					t.Fatal(err)
				}
				if err = db.Close(); err != nil {
					t.Fatal(err)
				}
				before, err := os.ReadFile(path)
				if err != nil {
					t.Fatal(err)
				}
				args := []string{command, "--db", path}
				if command == "trial" {
					args = append(args, "--mock", "--campaign", "foreign-db-check")
				}
				code, _, diagnostic := invoke(t, args...)
				if code != exitFailure || diagnostic["error"] != "database_open_failed" {
					t.Fatalf("code=%d diagnostic=%v", code, diagnostic)
				}
				after, err := os.ReadFile(path)
				if err != nil {
					t.Fatal(err)
				}
				if !bytes.Equal(before, after) {
					t.Fatal("unrelated database was changed")
				}
			})
		}
	}
}

func TestMockTrialsPersistForBothRoutesAndCases(t *testing.T) {
	for _, route := range []string{"mcp", "socket"} {
		for _, caseID := range []string{"color", "recovery"} {
			t.Run(route+"_"+caseID, func(t *testing.T) {
				path := filepath.Join(t.TempDir(), "bridge.db")
				code, trial, diagnostic := invoke(t, "trial", "--mock", "--db", path, "--campaign", "cli-test", "--path", route, "--case", caseID)
				if code != exitOK || string(trial["offline"]) != "true" || len(trial["report"]) == 0 {
					t.Fatalf("trial: code=%d output=%v diagnostic=%v", code, trial, diagnostic)
				}
				if _, err := os.Stat(path); err != nil {
					t.Fatalf("trial did not persist database: %v", err)
				}
				code, status, diagnostic := invoke(t, "status", "--db", path)
				var jobs []json.RawMessage
				if err := json.Unmarshal(status["jobs"], &jobs); err != nil || code != exitOK || len(jobs) == 0 {
					t.Fatalf("status: code=%d jobs=%v diagnostic=%v err=%v", code, jobs, diagnostic, err)
				}
				code, exported, diagnostic := invoke(t, "export", "--db", path)
				if code != exitOK || string(exported["offline"]) != "true" || len(exported["records"]) == 0 {
					t.Fatalf("export: code=%d output=%v diagnostic=%v", code, exported, diagnostic)
				}
				for i := 0; i < 2; i++ {
					code, stopped, diagnostic := invoke(t, "stop", "--db", path)
					if code != exitOK || string(stopped["stopped"]) != "true" || string(stopped["scope"]) != `"bridge_only"` || string(stopped["ai_processes_killed"]) != "false" {
						t.Fatalf("stop: code=%d output=%v diagnostic=%v", code, stopped, diagnostic)
					}
				}
				code, exported, diagnostic = invoke(t, "export", "--db", path)
				var records struct {
					Stopped bool `json:"stopped"`
				}
				if err := json.Unmarshal(exported["records"], &records); err != nil || code != exitOK || !records.Stopped {
					t.Fatalf("stop was not persisted: code=%d diagnostic=%v err=%v", code, diagnostic, err)
				}
			})
		}
	}
}

func TestMockTrialDefaults(t *testing.T) {
	path := filepath.Join(t.TempDir(), "defaults.db")
	code, out, diagnostic := invoke(t, "trial", "--mock", "--db", path, "--campaign", "defaults")
	if code != exitOK || string(out["offline"]) != "true" {
		t.Fatalf("code=%d output=%v diagnostic=%v", code, out, diagnostic)
	}
}

func TestErrorsAreBoundedAndDoNotEchoInput(t *testing.T) {
	secret := "PRIVATE\x1b[31m\n" + strings.Repeat("x", 1<<20)
	code, _, diagnostic := invoke(t, "trial", "--"+secret)
	if code != exitUsage || diagnostic["error"] != "invalid_arguments" {
		t.Fatalf("code=%d diagnostic=%v", code, diagnostic)
	}
	for _, value := range diagnostic {
		if strings.Contains(value, "PRIVATE") || strings.ContainsAny(value, "\x1b\n") {
			t.Fatalf("raw input leaked into diagnostic: %q", value)
		}
	}
}

type failingWriter struct{}

func (failingWriter) Write([]byte) (int, error) { return 0, errors.New("test writer failure") }

func TestOutputFailureReturnsNonzero(t *testing.T) {
	var stderr bytes.Buffer
	if code := run([]string{"help"}, failingWriter{}, &stderr); code != exitFailure {
		t.Fatalf("code=%d; want failure", code)
	}
	var diagnostic map[string]string
	if err := json.Unmarshal(stderr.Bytes(), &diagnostic); err != nil || diagnostic["error"] != "output_failed" {
		t.Fatalf("diagnostic=%v err=%v", diagnostic, err)
	}
}

func TestDiagnosticFailureStillReturnsNonzero(t *testing.T) {
	if code := run([]string{"live"}, io.Discard, failingWriter{}); code != exitUsage {
		t.Fatalf("code=%d; want usage failure", code)
	}
}
