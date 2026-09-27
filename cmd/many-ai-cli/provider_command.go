package main

import (
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/provider"
)

func runProviderCommand(cfg *config.Config, args []string) error {
	if len(args) == 0 {
		return errors.New("provider <backup|reset|recover>")
	}
	dir, err := config.Dir()
	if err != nil {
		return err
	}
	history, err := provider.NewHistoryStore(filepath.Join(dir, "provider-overrides"), filepath.Join(dir, "backups", "providers"))
	if err != nil {
		return err
	}
	switch args[0] {
	case "backup":
		return runProviderBackupCommand(history, args[1:])
	case "reset":
		return runProviderResetCommand(history, args[1:])
	case "recover":
		return runProviderRecoverCommand(history, args[1:], os.Stdout)
	default:
		return fmt.Errorf("unknown provider command: %s", args[0])
	}
}

func runProviderBackupCommand(history *provider.HistoryStore, args []string) error {
	if len(args) < 2 {
		return errors.New("provider backup <list|verify|restore> <provider-id> [backup-id]")
	}
	action, id := args[0], args[1]
	switch action {
	case "list":
		backups, err := history.ListBackups(id)
		if err != nil {
			return err
		}
		for _, backup := range backups {
			fmt.Printf("%s\t%s\t%s\t%s\n", backup.Revision, backup.CreatedAt, backup.Reason, backup.ContentDigest)
		}
		return nil
	case "verify":
		if len(args) < 3 {
			return errors.New("provider backup verify <provider-id> <backup-id>")
		}
		backup, err := history.VerifyBackup(id, args[2])
		if err != nil {
			return err
		}
		fmt.Printf("verified\t%s\t%s\n", backup.Revision, backup.ContentDigest)
		return nil
	case "restore":
		if len(args) < 3 {
			return errors.New("provider backup restore <provider-id> <backup-id> [--expected-revision <revision>]")
		}
		expected := ""
		for i := 3; i+1 < len(args); i++ {
			if args[i] == "--expected-revision" {
				expected = args[i+1]
			}
		}
		if expected == "" {
			return errors.New("--expected-revision is required")
		}
		revision, err := history.RestoreBackup(id, args[2], expected)
		if err != nil {
			return err
		}
		fmt.Printf("restored\t%s\n", revision.Revision)
		return nil
	default:
		return fmt.Errorf("unknown backup action: %s", action)
	}
}

func runProviderResetCommand(history *provider.HistoryStore, args []string) error {
	if len(args) < 2 || args[0] != "--distributed" {
		return errors.New("provider reset --distributed <provider-id> --expected-revision <revision>")
	}
	id := args[1]
	expected := ""
	hasExpected := false
	for i := 2; i+1 < len(args); i++ {
		if args[i] == "--expected-revision" {
			expected = args[i+1]
			hasExpected = true
		}
	}
	if !hasExpected {
		return errors.New("--expected-revision is required")
	}
	if !provider.IsBuiltinID(id) {
		return fmt.Errorf("built-in provider %q was not found", id)
	}
	revision, err := history.Reset(id, expected)
	if err != nil {
		return err
	}
	fmt.Printf("reset\t%s\n", revision.Revision)
	return nil
}

func runProviderRecoverCommand(history *provider.HistoryStore, args []string, out io.Writer) error {
	if len(args) < 1 {
		return errors.New("provider recover <provider-id> [revision|--list]")
	}
	id := args[0]
	if len(args) >= 2 && args[1] == "--list" {
		record, found, err := history.LastVerifiedRevision(id)
		if err != nil {
			return err
		}
		if found {
			fmt.Fprintf(out, "candidate\tuser_revision\t%s\t%s\n", record.Revision, record.CreatedAt)
		}
		fmt.Fprintln(out, "candidate\tbase")
		return nil
	}
	revision := ""
	if len(args) >= 2 {
		revision = args[1]
	}
	recovered, err := history.RecoverHead(id, revision)
	if err != nil {
		return err
	}
	fmt.Fprintf(out, "recovered\t%s\n", recovered.Revision)
	return nil
}
