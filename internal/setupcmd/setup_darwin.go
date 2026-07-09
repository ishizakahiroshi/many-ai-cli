//go:build darwin

package setupcmd

import (
	"fmt"
	"os"
	"path/filepath"
)

func createShortcuts(exe string) []Result {
	desktop := resolveMacDesktop()
	if desktop == "" {
		return []Result{{Path: "desktop", Err: fmt.Errorf("desktop directory not found")}}
	}
	if err := os.MkdirAll(desktop, 0o755); err != nil {
		return []Result{{Path: desktop, Err: fmt.Errorf("mkdir desktop: %w", err)}}
	}

	var results []Result

	start := filepath.Join(desktop, "Many AI Hub Start.command")
	if err := writeMacCommand(start, exe, "serve --open"); err != nil {
		results = append(results, Result{Path: start, Err: err})
	} else {
		results = append(results, Result{Path: start})
	}

	stop := filepath.Join(desktop, "Many AI Hub Stop.command")
	if err := writeMacCommand(stop, exe, "stop"); err != nil {
		results = append(results, Result{Path: stop, Err: err})
	} else {
		results = append(results, Result{Path: stop})
	}

	return results
}

// writeMacCommand は Finder ダブルクリックでターミナルが開いて実行される .command を作る。
// ローカル生成なので quarantine 属性は付かない。
func writeMacCommand(path, exe, args string) error {
	body := "#!/bin/sh\n" +
		"\"" + exe + "\" " + args + "\n"
	if err := os.WriteFile(path, []byte(body), 0o755); err != nil {
		return err
	}
	// WriteFile はプロセスの umask を通るため、確実に 0755 を当て直す。
	return os.Chmod(path, 0o755)
}

func resolveMacDesktop() string {
	if home, err := os.UserHomeDir(); err == nil {
		return filepath.Join(home, "Desktop")
	}
	return ""
}
