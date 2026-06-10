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

	"any-ai-cli/internal/config"
	"any-ai-cli/internal/hub"
	hublog "any-ai-cli/internal/log"
	"any-ai-cli/internal/sessionlog"
	"any-ai-cli/internal/shell"
	"any-ai-cli/internal/uninstall"
	"any-ai-cli/internal/usagerelay"
	"any-ai-cli/internal/wrapper"
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
				if strings.TrimSpace(line) == "module any-ai-cli" {
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
		// Windows GUI ランチャから引数なしで起動された場合でも hub.log にイベントが
		// 残るよう FileLogger を使う。wrap 経由で auto-spawn された場合は
		// CREATE_NEW_CONSOLE で新規コンソールが割り当てられるので、stderr 出力も
		// banner と同じ「Hub 専用ターミナル」に表示される。
		logger := hublog.NewFileLogger(cfg.Hub.LogDir, cfg.Log, false, true)
		s, err := hub.NewServer(cfg, logger, false, displayVersion())
		if err != nil {
			return err
		}
		s.SetAutoOpenBrowser(true)
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
	case "version", "--version", "-v":
		// deploy-wsl.ps1 など外部スクリプトが `any-ai-cli --version` /
		// `any-ai-cli version` で版数を取得できるようにする。
		// 出力は displayVersion() に一本化（ldflags 注入値 → git タグの順）。
		fmt.Println(displayVersion())
		return nil
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
		// serve 起動時は常に stderr にも slog を流す。
		// Hub 用コンソール（CREATE_NEW_CONSOLE で割り当てられた窓 or 直接起動された
		// シェル）でリアルタイムに動作状況を確認するため。
		logger = hublog.NewFileLogger(cfg.Hub.LogDir, cfg.Log, *debug, true)
		s, err := hub.NewServer(cfg, logger, *dev, displayVersion())
		if err != nil {
			return err
		}
		if *open {
			s.SetAutoOpenBrowser(true)
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
	case "log-clean":
		fs := flag.NewFlagSet("log-clean", flag.ContinueOnError)
		out := fs.String("o", "", "output transcript path")
		if err := fs.Parse(args[1:]); err != nil {
			return err
		}
		if fs.NArg() != 1 {
			return errors.New("log-clean <session.jsonl> [-o transcript.txt]")
		}
		jsonlPath := fs.Arg(0)
		outPath := *out
		if outPath == "" {
			ext := filepath.Ext(jsonlPath)
			outPath = strings.TrimSuffix(jsonlPath, ext) + ".txt"
		}
		if err := sessionlog.WriteTranscriptFile(jsonlPath, outPath); err != nil {
			return err
		}
		fmt.Println(outPath)
		return nil
	case "uninstall":
		fs := flag.NewFlagSet("uninstall", flag.ContinueOnError)
		purge := fs.Bool("purge", false, "バイナリ本体も削除する")
		if err := fs.Parse(args[1:]); err != nil {
			return err
		}
		if hub.IsRunning(cfg) {
			fmt.Println("Hub を停止中...")
			_ = hub.Stop(cfg)
		}
		return uninstall.Run(*purge)
	case "shell-init":
		fmt.Print(shell.InitScript())
		return nil
	case "wrap":
		if len(args) < 2 {
			return errors.New("wrap <provider>")
		}
		return wrapper.Run(cfg, logger, args[1], args[2:])
	case "claude", "codex", "copilot", "cursor-agent":
		return wrapper.Run(cfg, logger, cmd, args[1:])
	case "usage-relay":
		// 隠しサブコマンド: Claude statusLine / Codex Stop フックから呼び出される。
		// usage() ヘルプには載せない。
		return usagerelay.Run(args[1:])
	case "-h", "--help", "help":
		return usage()
	default:
		return fmt.Errorf("unknown command: %s", cmd)
	}
}

func usage() error {
	fmt.Println("any-ai-cli <serve|wrap|claude|codex|copilot|cursor-agent|shell-init|stop|status|log-clean|uninstall|version>")
	return nil
}
