package main

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"log/slog"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"strings"
	"syscall"

	"ai-cli-hub/internal/config"
	"ai-cli-hub/internal/hub"
	hublog "ai-cli-hub/internal/log"
	"ai-cli-hub/internal/shell"
	"ai-cli-hub/internal/wrapper"
)

// version はリリースビルド時に goreleaser の ldflags
// (-X main.version={{.Version}}) で git タグから注入される。
// これがバイナリ・Web UI・Windows メタデータ越しでバージョンを参照する
// single source of truth。
var version = "dev"

func main() {
	if err := run(os.Args[1:]); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func displayVersion() string {
	v := strings.TrimSpace(version)
	if v != "" && v != "dev" {
		return strings.TrimPrefix(v, "v")
	}
	for _, dir := range versionSourceDirs() {
		if tag := gitTagVersion(dir); tag != "" {
			return tag
		}
	}
	return "dev"
}

func versionSourceDirs() []string {
	var dirs []string
	if exe, err := os.Executable(); err == nil {
		dirs = append(dirs, filepath.Dir(exe))
	}
	if wd, err := os.Getwd(); err == nil {
		dirs = append(dirs, wd)
	}
	return dirs
}

func gitTagVersion(dir string) string {
	root := repoRoot(dir)
	if root == "" {
		return ""
	}
	cmd := exec.Command("git", "describe", "--tags", "--abbrev=0")
	cmd.Dir = root
	out, err := cmd.Output()
	if err != nil {
		return ""
	}
	tag := strings.TrimPrefix(strings.TrimSpace(string(out)), "v")
	if tag == "" {
		return ""
	}
	return tag
}

func repoRoot(dir string) string {
	for {
		data, err := os.ReadFile(filepath.Join(dir, "go.mod"))
		if err == nil {
			for _, line := range strings.Split(string(data), "\n") {
				if strings.TrimSpace(line) == "module ai-cli-hub" {
					return dir
				}
			}
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return ""
		}
		dir = parent
	}
}

func run(args []string) error {
	if len(args) == 0 {
		cfg, err := config.LoadOrCreate()
		if err != nil {
			return err
		}
		if hub.IsRunning(cfg) {
			_ = hub.OpenBrowserForConfig(cfg)
			return nil
		}
		logger := slog.New(slog.NewTextHandler(os.Stderr, nil))
		s, err := hub.NewServer(cfg, logger, false, displayVersion())
		if err != nil {
			return err
		}
		_ = s.OpenBrowser()
		ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
		defer stop()
		return s.Run(ctx)
	}
	cfg, err := config.LoadOrCreate()
	if err != nil {
		return err
	}
	logger := slog.New(slog.NewTextHandler(os.Stderr, nil))

	cmd := args[0]
	switch cmd {
	case "serve":
		fs := flag.NewFlagSet("serve", flag.ContinueOnError)
		open := fs.Bool("open", false, "open browser")
		port := fs.Int("port", 0, "port")
		dev := fs.Bool("dev", false, "serve web assets from ./web/ (no recompile needed)")
		debug := fs.Bool("debug", false, "enable debug logging")
		if err := fs.Parse(args[1:]); err != nil {
			return err
		}
		if *port > 0 {
			cfg.Hub.Port = *port
		}
		logger = hublog.NewFileLogger(cfg.Hub.LogDir, cfg.Log, *debug, *dev)
		s, err := hub.NewServer(cfg, logger, *dev, displayVersion())
		if err != nil {
			return err
		}
		if *open {
			_ = s.OpenBrowser()
		}
		ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
		defer stop()
		go func() {
			<-ctx.Done()
			logger.Info("shutdown signal received, shutting down")
		}()
		return s.Run(ctx)
	case "status":
		return hub.PrintStatus(cfg)
	case "stop":
		return hub.Stop(cfg)
	case "shell-init":
		fmt.Print(shell.InitScript())
		return nil
	case "wrap":
		if len(args) < 2 {
			return errors.New("wrap <provider>")
		}
		return wrapper.Run(cfg, logger, args[1], args[2:])
	case "claude", "codex", "gemini":
		return wrapper.Run(cfg, logger, cmd, args[1:])
	case "-h", "--help", "help":
		return usage()
	default:
		return fmt.Errorf("unknown command: %s", cmd)
	}
}

func usage() error {
	fmt.Println("ai-cli-hub <serve|wrap|claude|codex|gemini|shell-init|stop|status>")
	return nil
}
