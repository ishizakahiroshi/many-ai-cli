package main

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"log/slog"
	"os"
	"os/signal"
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
		s, err := hub.NewServer(cfg, logger, false, version)
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
		s, err := hub.NewServer(cfg, logger, *dev, version)
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
