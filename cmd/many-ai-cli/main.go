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
	"runtime/debug"
	"strings"
	"syscall"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/doctor"
	"many-ai-cli/internal/hub"
	"many-ai-cli/internal/launcher"
	hublog "many-ai-cli/internal/log"
	"many-ai-cli/internal/orchestrate"
	"many-ai-cli/internal/sessionlog"
	"many-ai-cli/internal/setupcmd"
	"many-ai-cli/internal/shell"
	"many-ai-cli/internal/uninstall"
	"many-ai-cli/internal/tray"
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

// waitForShutdownSignal は SIGINT/SIGTERM を待ち受ける context を返す。
// 起動ログ（"MANY-AI-CLI started"）と対になる終了ログを、シグナルが実際に
// 届いた時点で reason="signal" ＋ 具体的なシグナル名付きで残す
//（plan_hub-lifecycle-logging.md C1）。返り値の cancel は defer で必ず呼ぶこと
// （シグナル未着のまま return するパスでも goroutine と signal.Notify 登録を解放する）。
func waitForShutdownSignal(logger *slog.Logger, instanceID string) (context.Context, context.CancelFunc) {
	ctx, cancel := context.WithCancel(context.Background())
	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, os.Interrupt, syscall.SIGTERM)
	go func() {
		select {
		case sig := <-sigCh:
			logger.Info("MANY-AI-CLI stopping", "reason", "signal", "signal", sig.String(), "pid", os.Getpid(), "instance_id", instanceID)
			cancel()
		case <-ctx.Done():
		}
	}()
	return ctx, func() {
		cancel()
		signal.Stop(sigCh)
	}
}

func main() {
	// トップレベルの panic recovery（plan_hub-lifecycle-logging.md C4）。
	// internal/hub/safe_go.go の safeGo はバックグラウンド goroutine 単位の
	// panic しか拾わない設計のため、main の実行パス自体（serve のブロッキング
	// Run() 呼び出し含む）で起きた panic はここで拾ってスタックトレース込みで
	// hub.log に記録してから異常終了する。recover してプロセスを継続させる
	// 必要はない（記録できれば目的は達成）。
	defer func() {
		if r := recover(); r != nil {
			logPanic(r)
			os.Exit(1)
		}
	}()
	if err := run(os.Args[1:]); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

// logPanic writes a panic + stack trace to hub.log (falling back to stderr
// only if the config/log dir can't be resolved at all) so a crash leaves a
// diagnostic trail instead of vanishing silently
// (plan_hub-lifecycle-logging.md C4).
func logPanic(r any) {
	logDir := ""
	var logCfg config.LogConfig
	if cfg, err := config.LoadOrCreate(); err == nil {
		logDir = cfg.Hub.LogDir
		logCfg = cfg.Log
	}
	logger := hublog.NewFileLogger(logDir, logCfg, false, true)
	logger.Error("MANY-AI-CLI panic",
		"recover", fmt.Sprintf("%v", r),
		"stack", string(debug.Stack()),
		"pid", os.Getpid(),
	)
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
		ctx, stop := waitForShutdownSignal(logger, s.InstanceID())
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
		ctx, stop := waitForShutdownSignal(logger, s.InstanceID())
		defer stop()
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
	case "tray":
		// Windows のデスクトップにある起動用と停止用のアイコン 2 個を、トレイ 1 個へ
		// まとめるための常駐プロセス。窓は作らず、UI は既定ブラウザのタブで開く。
		// Windows 以外は tray.ErrUnsupported を返す。
		return tray.Run(cfg)
	case "doctor":
		fs := flag.NewFlagSet("doctor", flag.ContinueOnError)
		asJSON := fs.Bool("json", false, "output as JSON")
		if err := fs.Parse(args[1:]); err != nil {
			return err
		}
		report := doctor.Run(context.Background(), cfg)
		if *asJSON {
			return json.NewEncoder(os.Stdout).Encode(report)
		}
		for _, check := range report.Checks {
			fmt.Printf("[%s] %s: %s\n", check.Level, check.Name, check.Message)
			if check.Fix != "" {
				fmt.Printf("      -> %s\n", check.Fix)
			}
		}
		return nil
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
	case "setup":
		fs := flag.NewFlagSet("setup", flag.ContinueOnError)
		if err := fs.Parse(args[1:]); err != nil {
			if errors.Is(err, flag.ErrHelp) {
				return nil
			}
			return err
		}
		return setupcmd.Run()
	case "issue":
		return runIssue(cfg, args[1:], defaultIssueDependencies())
	case "wrap":
		if len(args) < 2 {
			return errors.New("wrap <provider>")
		}
		return wrapper.Run(cfg, logger, args[1], args[2:])
	case "claude", "codex", "copilot", "cursor-agent", "opencode", "grok":
		return wrapper.Run(cfg, logger, cmd, args[1:])
	case "usage-relay":
		// 隠しサブコマンド: Claude statusLine / Codex Stop フックから呼び出される。
		// usage() ヘルプには載せない。
		return usagerelay.Run(args[1:])
	case "orchestrate":
		// 隠しサブコマンド: orchestration conductor / child セッションの AI が
		// `many-ai-cli orchestrate spawn --role ... "prompt"`（子の起動）と
		// `many-ai-cli orchestrate send --role ... "text"`（既存子への追加指示）に使う
		// （plan_orchestration-spawn-ui-exposure.md C2 / plan_orchestration-conductor-improvements.md C2）。
		// usage() ヘルプには載せない。
		if len(args) < 2 {
			return errors.New("orchestrate <spawn|send>")
		}
		return orchestrate.Run(args[1:])
	case "-h", "--help", "help":
		return usage()
	default:
		return fmt.Errorf("unknown command: %s", cmd)
	}
}

func usage() error {
	fmt.Println("many-ai-cli <serve|connect|setup|doctor|issue|wrap|claude|codex|copilot|cursor-agent|opencode|grok|shell-init|stop|status|tray|profile-export|log-clean|uninstall|version>")
	return nil
}
