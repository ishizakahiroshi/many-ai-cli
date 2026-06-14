// many-ai-cli-launcher is the unified launcher for many-ai-cli remote
// connections. It reads connection profiles from
// ~/.many-ai-cli/launcher-profiles.yaml and connects to a Hub via WSL (Windows
// only) or SSH.
//
// Usage:
//
//	many-ai-cli-launcher [--profile <name>] [--last] [--ui]
//
// Without flags (= plain double-click / direct invocation) a browser-based
// profile selection page is always opened on a random loopback port;
// already-connected profiles are marked there. Direct connection without the
// UI requires --profile or --last (e.g. a dedicated shortcut).
package main

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"syscall"

	"many-ai-cli/internal/launcher"
)

// version is injected at release build time via
// -ldflags "-X main.version=..." (see .goreleaser.yaml). Defaults to "dev"
// for local builds, mirroring cmd/many-ai-cli.
var version = "dev"

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run() error {
	launcher.ConfigureConsoleUTF8()
	fmt.Fprint(os.Stdout, launcher.StartupBanner(version))

	fs := flag.NewFlagSet("many-ai-cli-launcher", flag.ContinueOnError)
	profileName := fs.String("profile", "", "profile name to connect (see ~/.many-ai-cli/launcher-profiles.yaml)")
	useLast := fs.Bool("last", false, "connect using the last-used profile")
	openUI := fs.Bool("ui", false, "open the profile selection UI in the browser")
	if err := fs.Parse(os.Args[1:]); err != nil {
		if errors.Is(err, flag.ErrHelp) {
			return nil
		}
		return err
	}

	pf, err := launcher.LoadProfiles()
	if err != nil {
		return fmt.Errorf("load profiles: %w", err)
	}
	if err := launcher.Validate(pf); err != nil {
		return fmt.Errorf("invalid profiles: %w", err)
	}

	// No flags (= plain double-click) always opens the selection UI so the
	// user can pick / add / edit profiles and see already-running
	// connections. Direct connect requires an explicit --profile / --last.
	if *openUI || (*profileName == "" && !*useLast) {
		return runUI()
	}

	profile, err := launcher.SelectProfile(pf, *profileName, *useLast)
	if err != nil {
		return err
	}

	return launcher.Connect(profile)
}

// runUI starts the local HTTP server for the profile selection page, opens the
// default browser, and waits until the user closes the window or Ctrl-C.
func runUI() error {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	srv, err := launcher.NewUIServer()
	if err != nil {
		return fmt.Errorf("create ui server: %w", err)
	}

	pageURL, err := srv.Serve(ctx)
	if err != nil {
		return fmt.Errorf("start ui server: %w", err)
	}

	fmt.Fprintf(os.Stdout, "Opening connection selection page: %s\n", pageURL)
	launcher.OpenBrowserOnce(pageURL)

	// Wait until Ctrl-C or the OS sends SIGTERM.
	<-ctx.Done()
	// このプロセスが抱えていたトンネル / serve はここで全て死ぬので、
	// launcher-active.json の自プロセス分を掃除する（強制終了時の残骸は
	// 読み取り側の二重ガードで除外される）。
	_ = launcher.UnregisterAllForPID()
	return nil
}

// 接続フロー（connect / selectProfile）は internal/launcher パッケージへ移設し、
// 本体の `many-ai-cli connect` サブコマンドと共用している
// （launcher.Connect / launcher.SelectProfile）。
