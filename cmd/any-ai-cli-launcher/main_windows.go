//go:build windows

// any-ai-cli-launcher is the unified Windows launcher for any-ai-cli remote
// connections. It reads connection profiles from
// ~/.any-ai-cli/launcher-profiles.yaml and connects to a Hub via WSL or SSH.
//
// Usage:
//
//	any-ai-cli-launcher [--profile <name>] [--last] [--ui]
//
// If exactly one profile is defined it is used without flags.
// When multiple profiles are present and no flag is given, or when --ui is
// specified, a browser-based profile selection page is opened on a random
// loopback port.
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

// errOpenUI is a sentinel error returned by selectProfile to indicate that
// the UI selection page should be opened instead of connecting directly.
var errOpenUI = errors.New("open_ui")

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run() error {
	launcher.ConfigureConsoleUTF8()

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

	// --ui flag or no profiles → open the selection UI.
	if *openUI || len(pf.Profiles) == 0 {
		return runUI()
	}

	profile, err := selectProfile(pf, *profileName, *useLast)
	if errors.Is(err, errOpenUI) {
		return runUI()
	}
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

	fmt.Fprintf(os.Stdout, "接続先選択画面を開いています: %s\n", pageURL)
	launcher.OpenBrowserOnce(pageURL)

	// Wait until Ctrl-C or the OS sends SIGTERM.
	<-ctx.Done()
	return nil
}

// connect runs the connection flow for a known profile.
func connect(profile launcher.Profile) error {
	conn, err := connectorFor(profile)
	if err != nil {
		return err
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	urlCh := make(chan string, 1)
	errCh := make(chan error, 1)

	if err := conn.Start(ctx, profile, urlCh, errCh); err != nil {
		return err
	}

	var wg sync.WaitGroup
	wg.Add(1)
	go func() {
		defer wg.Done()
		select {
		case url, ok := <-urlCh:
			if ok && url != "" {
				launcher.OpenBrowserOnce(url)
			}
		case <-ctx.Done():
		}
	}()

	select {
	case err := <-errCh:
		stop()
		wg.Wait()
		if err != nil {
			return err
		}
	case <-ctx.Done():
		wg.Wait()
	}

	return nil
}

// selectProfile chooses which profile to connect based on the CLI flags and
// the number of profiles available.
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

	switch len(pf.Profiles) {
	case 1:
		// Exactly one profile: connect immediately without flags.
		return pf.Profiles[0], nil
	default:
		// Multiple profiles and no flag: signal to open the selection UI.
		return launcher.Profile{}, errOpenUI
	}
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
