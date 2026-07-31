//go:build linux

package setupcmd

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

func createShortcuts(exe string) []Result {
	var results []Result

	appsDir := filepath.Join(userHome(), ".local", "share", "applications")
	if err := os.MkdirAll(appsDir, 0o755); err != nil {
		results = append(results, Result{Path: appsDir, Err: fmt.Errorf("mkdir: %w", err)})
		return results
	}

	entries := []struct {
		file string
		name string
		args string
	}{
		{"many-ai-hub-start.desktop", "Many AI Hub Start", "serve --open"},
		{"many-ai-hub-stop.desktop", "Many AI Hub Stop", "stop"},
	}

	desktop := resolveLinuxDesktop()

	for _, e := range entries {
		appPath := filepath.Join(appsDir, e.file)
		if err := writeLinuxDesktop(appPath, exe, e.name, e.args); err != nil {
			results = append(results, Result{Path: appPath, Err: err})
			continue
		}
		results = append(results, Result{Path: appPath})

		if desktop == "" {
			continue
		}
		dupPath := filepath.Join(desktop, e.file)
		if err := writeLinuxDesktop(dupPath, exe, e.name, e.args); err != nil {
			results = append(results, Result{Path: dupPath, Err: err})
			continue
		}
		results = append(results, Result{Path: dupPath})
	}

	return results
}

func writeLinuxDesktop(path, exe, name, args string) error {
	exe = strings.ReplaceAll(exe, "%", "%%")
	body := "[Desktop Entry]\n" +
		"Type=Application\n" +
		"Name=" + name + "\n" +
		"Exec=\"" + exe + "\" " + args + "\n" +
		"Terminal=true\n" +
		"Categories=Development;\n"
	if err := os.WriteFile(path, []byte(body), 0o755); err != nil {
		return err
	}
	return os.Chmod(path, 0o755)
}

func resolveLinuxDesktop() string {
	if out, err := exec.Command("xdg-user-dir", "DESKTOP").Output(); err == nil {
		p := strings.TrimSpace(string(out))
		if p != "" {
			if _, err := os.Stat(p); err == nil {
				return p
			}
		}
	}
	p := filepath.Join(userHome(), "Desktop")
	if _, err := os.Stat(p); err == nil {
		return p
	}
	return ""
}

func userHome() string {
	h, _ := os.UserHomeDir()
	return h
}
