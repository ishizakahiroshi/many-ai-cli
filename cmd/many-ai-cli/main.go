package main

import (
	"context"
	"encoding/json"
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

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/hub"
	"many-ai-cli/internal/launcher"
	hublog "many-ai-cli/internal/log"
	"many-ai-cli/internal/sessionlog"
	"many-ai-cli/internal/shell"
	"many-ai-cli/internal/uninstall"
	"many-ai-cli/internal/usagerelay"
	"many-ai-cli/internal/wrapper"
)

// version はリリースビルド時に goreleaser の ldflags
// (-X main.version={{.Version}}) で git タグから注入される。
// これがバイナリ・Web UI・Windows メタデータ越しでバージョンを参照する
// single source of truth。
var version = "dev"

// gitCommit / buildTime はビルド時に ldflags (-X main.gitCommit=... /
// -X main.buildTime=...) で注入される人間可読のビルド識別子。同一 version 内の
// ビルド差を識別するための付加情報で、/api/info に出す。未注入なら空文字。
var (
	gitCommit = ""
	buildTime = ""
)

// buildInfo は Hub へ渡すビルド識別子をまとめる。
func buildInfo() hub.BuildInfo {
	return hub.BuildInfo{GitCommit: gitCommit, BuildTime: buildTime}
}

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
				if strings.TrimSpace(line) == "module many-ai-cli" {
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
		s, err := hub.NewServer(cfg, logger, false, displayVersion(), buildInfo())
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
		// deploy-wsl.ps1 など外部スクリプトが `many-ai-cli --version` /
		// `many-ai-cli version` で版数を取得できるようにする。
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
		s, err := hub.NewServer(cfg, logger, *dev, displayVersion(), buildInfo())
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
	case "connect":
		// リモート Hub へターミナルから接続する（SmartScreen フォールバックの正規手順）。
		// 別 exe many-ai-cli-launcher のダブルクリックを使わずに済むよう、同じ接続
		// フロー（internal/launcher）を本体サブコマンドとして公開する。
		fs := flag.NewFlagSet("connect", flag.ContinueOnError)
		profileName := fs.String("profile", "", "profile name to connect (see ~/.many-ai-cli/launcher-profiles.yaml)")
		useLast := fs.Bool("last", false, "connect using the last-used profile")
		if err := fs.Parse(args[1:]); err != nil {
			if errors.Is(err, flag.ErrHelp) {
				return nil
			}
			return err
		}
		if *profileName == "" && !*useLast {
			return errors.New("connect requires --profile <name> or --last")
		}
		launcher.ConfigureConsoleUTF8()
		pf, err := launcher.LoadProfiles()
		if err != nil {
			return fmt.Errorf("load profiles: %w", err)
		}
		if err := launcher.Validate(pf); err != nil {
			return fmt.Errorf("invalid profiles: %w", err)
		}
		profile, err := launcher.SelectProfile(pf, *profileName, *useLast)
		if err != nil {
			return err
		}
		return launcher.Connect(profile)
	case "profile-export":
		// リモートサーバー上で実行し、自分へ SSH 接続するための接続プロファイルを
		// 鍵を除いて JSON 出力する。手元 PC の UI が SSH-pull でこれを取得して
		// 接続フォームを自動補完する（plan_server-profile-export-import.md C1）。
		fs := flag.NewFlagSet("profile-export", flag.ContinueOnError)
		asJSON := fs.Bool("json", false, "output as JSON")
		name := fs.String("name", "", "profile display name (default: hostname)")
		host := fs.String("host", "", "public host to advertise first (overrides auto-detected IP; e.g. for Docker)")
		cwd := fs.String("cwd", "", "remote working directory (default: current directory)")
		hubPort := fs.Int("hub-port", 0, "fixed hub port (0 = auto-select)")
		if err := fs.Parse(args[1:]); err != nil {
			if errors.Is(err, flag.ErrHelp) {
				return nil
			}
			return err
		}
		if !*asJSON {
			return errors.New("profile-export currently supports only --json output")
		}
		exported := launcher.BuildExportProfile(launcher.ExportOptions{
			Name:       *name,
			PublicHost: *host,
			CWD:        *cwd,
			HubPort:    *hubPort,
		})
		enc := json.NewEncoder(os.Stdout)
		enc.SetIndent("", "  ")
		return enc.Encode(exported)
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
	case "claude", "codex", "copilot", "cursor-agent", "opencode":
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
	fmt.Println("many-ai-cli <serve|connect|wrap|claude|codex|copilot|cursor-agent|opencode|shell-init|stop|status|profile-export|log-clean|uninstall|version>")
	return nil
}
