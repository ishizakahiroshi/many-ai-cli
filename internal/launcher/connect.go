package launcher

import (
	"context"
	"fmt"
	"os"
	"os/signal"
	"sync"
	"syscall"
	"time"
)

// Connect runs the connection flow for a known profile. It reuses an existing
// live connection if one is found, otherwise starts the SSH/WSL connector,
// opens the browser when the Hub URL arrives, and blocks until the connection
// ends or the process is interrupted (Ctrl-C / SIGTERM).
//
// The calling process owns the tunnel for its entire lifetime — closing the
// terminal / Ctrl-C tears it down (see CloseBehaviorNotice). Both the
// standalone launcher exe and `many-ai-cli connect` call this so the flow lives
// in one place.
func Connect(profile Profile) error {
	// 多重起動ガード: 同一プロファイルが既に接続中（PID 生存 + Hub 応答の二重
	// ガード済み）なら、新しい serve / トンネルを張らずに既存の Hub URL でブラウザを
	// 開いて終了する。確認失敗時は best-effort で通常接続に進む。
	if conns, err := ActiveConnectionsPruned(); err == nil {
		for _, c := range conns {
			if c.Profile == profile.Name {
				fmt.Fprintf(os.Stdout, "Profile %q is already connected — reusing %s\n", profile.Name, c.HubURL)
				OpenBrowserOnce(c.HubURL)
				return nil
			}
		}
	}

	startupLock, acquired, err := TryAcquireProfileConnectLock(profile.Name)
	if err != nil {
		return fmt.Errorf("acquire startup lock: %w", err)
	}
	if !acquired {
		fmt.Fprintf(os.Stdout, "Profile %q is already starting — waiting for Hub URL...\n", profile.Name)
		if c, ok := WaitForActiveConnection(profile.Name, 30*time.Second); ok {
			fmt.Fprintf(os.Stdout, "Profile %q is connected — reusing %s\n", profile.Name, c.HubURL)
			OpenBrowserOnce(c.HubURL)
			return nil
		}
		fmt.Fprintf(os.Stdout, "Profile %q is still starting; no new terminal was opened.\n", profile.Name)
		return nil
	}
	defer func() { _ = startupLock.Release() }()

	conn, err := ConnectorFor(profile)
	if err != nil {
		return err
	}
	fmt.Fprint(os.Stdout, CloseBehaviorNotice(profile))

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	urlCh := make(chan string, 1)
	errCh := make(chan error, 1)

	if err := conn.Start(ctx, profile, urlCh, errCh); err != nil {
		return err
	}
	// 接続記録は URL 受信時に登録されるため、終了時は無条件に削除してよい（未登録なら no-op）。
	defer func() { _ = UnregisterActiveConnection(profile.Name) }()

	var wg sync.WaitGroup
	wg.Add(1)
	go func() {
		defer wg.Done()
		select {
		case url, ok := <-urlCh:
			if ok && url != "" {
				OpenBrowserOnce(url)
				// 他のランチャープロセス（選択 UI / Hub の Server ボタン）から
				// 「接続中」と見えるように記録する。
				if err := RegisterActiveConnection(profile.Name, url); err != nil {
					fmt.Fprintf(os.Stderr, "failed to record active connection: %v\n", err)
				}
				_ = startupLock.Release()
			}
		case <-ctx.Done():
		}
	}()

	select {
	case err, ok := <-errCh:
		if !ok {
			// errCh の close は「接続終了」（リモート serve 停止 / トンネル切断 /
			// wsl.exe 正常終了）。プロセスも終了してコンソール窓を閉じる（残骸防止）。
			fmt.Fprintln(os.Stdout, "Connection closed — exiting.")
			stop()
			wg.Wait()
			return nil
		}
		stop()
		wg.Wait()
		return err
	case <-ctx.Done():
		wg.Wait()
		return nil
	}
}

// SelectProfile chooses which profile to connect based on the given name /
// useLast flags. Exactly one of name or useLast must select a profile.
func SelectProfile(pf *ProfilesFile, name string, useLast bool) (Profile, error) {
	if name != "" {
		return findByName(pf, name)
	}
	if useLast {
		if pf.LastUsed == "" {
			return Profile{}, fmt.Errorf("no last-used profile recorded in launcher-profiles.yaml")
		}
		return findByName(pf, pf.LastUsed)
	}
	return Profile{}, fmt.Errorf("SelectProfile requires a profile name or --last")
}

func findByName(pf *ProfilesFile, name string) (Profile, error) {
	for _, p := range pf.Profiles {
		if p.Name == name {
			return p, nil
		}
	}
	return Profile{}, fmt.Errorf("profile %q not found in launcher-profiles.yaml", name)
}
