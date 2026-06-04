//go:build windows

// any-ai-cli-launcher is the unified Windows launcher for any-ai-cli remote
// connections. It reads connection profiles from
// ~/.any-ai-cli/launcher-profiles.yaml and connects to a Hub via WSL or SSH.
//
// Usage:
//
//	any-ai-cli-launcher [--profile <name>] [--last] [--ui]
//
// Without flags (= plain double-click) a browser-based profile selection
// page is always opened on a random loopback port; already-connected
// profiles are marked there. Direct connection without the UI requires
// --profile or --last (e.g. a dedicated shortcut).
package main

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"sync"
	"syscall"

	"any-ai-cli/internal/launcher"
)

// version is injected at release build time via
// -ldflags "-X main.version=..." (see .goreleaser.yaml). Defaults to "dev"
// for local builds, mirroring cmd/any-ai-cli.
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

	fs := flag.NewFlagSet("any-ai-cli-launcher", flag.ContinueOnError)
	profileName := fs.String("profile", "", "profile name to connect (see ~/.any-ai-cli/launcher-profiles.yaml)")
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

	profile, err := selectProfile(pf, *profileName, *useLast)
	if err != nil {
		return err
	}

	return connect(profile)
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

// connect runs the connection flow for a known profile.
func connect(profile launcher.Profile) error {
	conn, err := connectorFor(profile)
	if err != nil {
		return err
	}
	fmt.Fprint(os.Stdout, launcher.CloseBehaviorNotice(profile))

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	urlCh := make(chan string, 1)
	errCh := make(chan error, 1)

	if err := conn.Start(ctx, profile, urlCh, errCh); err != nil {
		return err
	}
	// 接続記録は URL 受信時に登録されるため、終了時は無条件に削除してよい
	//（未登録なら no-op）。
	defer func() { _ = launcher.UnregisterActiveConnection(profile.Name) }()

	var wg sync.WaitGroup
	wg.Add(1)
	go func() {
		defer wg.Done()
		select {
		case url, ok := <-urlCh:
			if ok && url != "" {
				launcher.OpenBrowserOnce(url)
				// 他のランチャープロセス（選択 UI）から「接続中」と
				// 見えるように記録する。
				if err := launcher.RegisterActiveConnection(profile.Name, url); err != nil {
					fmt.Fprintf(os.Stderr, "any-ai-cli-launcher: failed to record active connection: %v\n", err)
				}
			}
		case <-ctx.Done():
		}
	}()

	for {
		select {
		case err, ok := <-errCh:
			if !ok {
				// コネクタは接続成功時に errCh を close する（エラー送信なし）。
				// close をエラーと同じ扱いで return すると、トンネル確立直後に
				// プロセスごと終了して defer がトンネルを殺し、開いたブラウザが
				// ERR_CONNECTION_REFUSED になる。閉鎖は「成功・継続中」の意味
				// なので、以降は Ctrl+C / シグナルまで待ち続ける。
				errCh = nil
				continue
			}
			stop()
			wg.Wait()
			return err
		case <-ctx.Done():
			wg.Wait()
			return nil
		}
	}
}

// selectProfile chooses which profile to connect based on the CLI flags.
// The caller guarantees that name or useLast is set (the no-flag case opens
// the selection UI before this is reached).
func selectProfile(pf *launcher.ProfilesFile, name string, useLast bool) (launcher.Profile, error) {
	if name != "" {
		return findByName(pf, name)
	}
	if useLast {
		if pf.LastUsed == "" {
			return launcher.Profile{}, fmt.Errorf("no last-used profile recorded in launcher-profiles.yaml")
		}
		return findByName(pf, pf.LastUsed)
	}
	return launcher.Profile{}, fmt.Errorf("selectProfile requires --profile or --last")
}

func findByName(pf *launcher.ProfilesFile, name string) (launcher.Profile, error) {
	for _, p := range pf.Profiles {
		if p.Name == name {
			return p, nil
		}
	}
	return launcher.Profile{}, fmt.Errorf("profile %q not found in launcher-profiles.yaml", name)
}

// connectorFor returns the correct Connector for the given profile type.
func connectorFor(p launcher.Profile) (launcher.Connector, error) {
	switch p.Type {
	case launcher.ProfileTypeWSL:
		return launcher.NewWSLConnector(), nil
	case launcher.ProfileTypeSSH:
		return launcher.NewSSHConnector(), nil
	default:
		return nil, fmt.Errorf("unsupported profile type %q", p.Type)
	}
}
