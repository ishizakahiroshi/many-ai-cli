//go:build windows

package wrapper

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"unsafe"

	"golang.org/x/sys/windows"
)

var (
	versionDLL                  = syscall.NewLazyDLL("version.dll")
	procGetFileVersionInfoSizeW = versionDLL.NewProc("GetFileVersionInfoSizeW")
	procGetFileVersionInfoW     = versionDLL.NewProc("GetFileVersionInfoW")
	procVerQueryValueW          = versionDLL.NewProc("VerQueryValueW")
)

type vsFixedFileInfo struct {
	Signature        uint32
	StrucVersion     uint32
	FileVersionMS    uint32
	FileVersionLS    uint32
	ProductVersionMS uint32
	ProductVersionLS uint32
	FileFlagsMask    uint32
	FileFlags        uint32
	FileOS           uint32
	FileType         uint32
	FileSubtype      uint32
	FileDateMS       uint32
	FileDateLS       uint32
}

const processQueryLimitedInformation = 0x1000
const parentShellEnv = "AI_CLI_HUB_PARENT_SHELL"

// processExeFullPath は PID からプロセスのフルパスを返す。
func processExeFullPath(pid uint32) string {
	handle, err := windows.OpenProcess(processQueryLimitedInformation, false, pid)
	if err != nil {
		return ""
	}
	defer windows.CloseHandle(handle)
	buf := make([]uint16, windows.MAX_PATH)
	size := uint32(len(buf))
	if err := windows.QueryFullProcessImageName(handle, 0, &buf[0], &size); err != nil {
		return ""
	}
	return windows.UTF16ToString(buf[:size])
}

// getExeFileVersion は実行ファイルのファイルバージョンを "major.minor.patch" 形式で返す。
func getExeFileVersion(exePath string) string {
	pathPtr, err := syscall.UTF16PtrFromString(exePath)
	if err != nil {
		return ""
	}
	var dummy uint32
	size, _, _ := procGetFileVersionInfoSizeW.Call(uintptr(unsafe.Pointer(pathPtr)), uintptr(unsafe.Pointer(&dummy)))
	if size == 0 {
		return ""
	}
	buf := make([]byte, size)
	ret, _, _ := procGetFileVersionInfoW.Call(uintptr(unsafe.Pointer(pathPtr)), 0, size, uintptr(unsafe.Pointer(&buf[0])))
	if ret == 0 {
		return ""
	}
	subBlock, _ := syscall.UTF16PtrFromString(`\`)
	var info *vsFixedFileInfo
	var infoLen uint32
	ret, _, _ = procVerQueryValueW.Call(
		uintptr(unsafe.Pointer(&buf[0])),
		uintptr(unsafe.Pointer(subBlock)),
		uintptr(unsafe.Pointer(&info)),
		uintptr(unsafe.Pointer(&infoLen)),
	)
	if ret == 0 || infoLen == 0 {
		return ""
	}
	major := (info.FileVersionMS >> 16) & 0xffff
	minor := info.FileVersionMS & 0xffff
	patch := (info.FileVersionLS >> 16) & 0xffff
	return fmt.Sprintf("%d.%d.%d", major, minor, patch)
}

// shellVersionSuffix は exe 名と PID からバージョン文字列を返す。
// pwsh.exe → "7.4.2"、powershell.exe → "5.1"（ビルド番号は省略）
func shellVersionSuffix(exe string, pid uint32) string {
	switch strings.ToLower(exe) {
	case "powershell.exe":
		// Windows PowerShell は 5.1 で凍結済み。FileVersion は OS ビルド由来で不正確。
		return "5.1"
	case "pwsh.exe":
		exePath := processExeFullPath(pid)
		if exePath == "" {
			return ""
		}
		ver := getExeFileVersion(exePath)
		if ver == "" {
			return ""
		}
		return ver
	}
	return ""
}

// DetectShell は起動元シェルの種別を返す。
// AI_CLI_HUB_PARENT_SHELL がセットされている場合はその値を最優先で返す。
// CreateToolhelp32Snapshot で親プロセスの exe 名を取得し、フレンドリー名に変換する。
// /api/spawn 経由で起動した場合は直親が ai-cli-hub.exe になるため、その場合はさらに祖父プロセスを参照する。
func DetectShell() string {
	if v := strings.TrimSpace(os.Getenv(parentShellEnv)); v != "" {
		return v
	}

	pid := uint32(os.Getppid())
	for pid != 0 {
		exe, grandpid := processExeAndParent(pid)
		if exe == "" {
			break
		}
		if shell, ok := knownWindowsShellName(exe); ok {
			if ver := shellVersionSuffix(exe, pid); ver != "" {
				return shell + " " + ver
			}
			return shell
		}
		if shouldSkipWindowsShellProcess(exe) {
			pid = grandpid
			continue
		}
		return windowsExeToShell(exe)
	}
	return fallbackShellWindows()
}

func parentProcessExe(ppid uint32) string {
	exe, _ := processExeAndParent(ppid)
	return exe
}

// processExeAndParent は pid に対応するプロセスの exe 名と親 PID を返す。
func processExeAndParent(pid uint32) (exe string, ppid uint32) {
	snap, err := windows.CreateToolhelp32Snapshot(windows.TH32CS_SNAPPROCESS, 0)
	if err != nil {
		return "", 0
	}
	defer windows.CloseHandle(snap)

	var entry windows.ProcessEntry32
	entry.Size = uint32(unsafe.Sizeof(entry))
	if err := windows.Process32First(snap, &entry); err != nil {
		return "", 0
	}
	for {
		if entry.ProcessID == pid {
			return windows.UTF16ToString(entry.ExeFile[:]), entry.ParentProcessID
		}
		if err := windows.Process32Next(snap, &entry); err != nil {
			break
		}
	}
	return "", 0
}

func windowsExeToShell(exe string) string {
	if shell, ok := knownWindowsShellName(exe); ok {
		return shell
	}
	// 未知の .exe はサフィックスを除いて返す
	if strings.HasSuffix(strings.ToLower(exe), ".exe") {
		return exe[:len(exe)-4]
	}
	return exe
}

func knownWindowsShellName(exe string) (string, bool) {
	switch strings.ToLower(exe) {
	case "pwsh.exe":
		return "PowerShell 7", true
	case "powershell.exe":
		return "Windows PowerShell", true
	case "cmd.exe":
		return "cmd", true
	case "bash.exe":
		return "bash", true
	case "zsh.exe":
		return "zsh", true
	case "nu.exe":
		return "nushell", true
	case "wsl.exe":
		return "WSL", true
	}
	return "", false
}

func shouldSkipWindowsShellProcess(exe string) bool {
	switch strings.ToLower(exe) {
	case "ai-cli-hub.exe",
		"explorer.exe",
		"windowsterminal.exe",
		"wt.exe",
		"conhost.exe",
		"openconsole.exe",
		// ファイルマネージャー・ランチャー系（シェルではないが親プロセスになり得る）
		"onecommander.exe",
		"totalcmd.exe",
		"totalcmd64.exe",
		"doublecmd.exe",
		"freecommander.exe",
		"multicommander.exe":
		return true
	}
	return false
}

// fallbackShellWindows は PPID 取得に失敗した場合の代替検出。
// PSModulePath 環境変数でバージョンを判定する。
func fallbackShellWindows() string {
	modPath := os.Getenv("PSModulePath")
	if modPath == "" {
		return ""
	}
	norm := strings.ToLower(filepath.ToSlash(modPath))
	if strings.Contains(norm, "microsoft.powershell_7") || strings.Contains(norm, "/powershell/7/") {
		return "PowerShell 7"
	}
	if strings.Contains(norm, "microsoft.powershell_6") || strings.Contains(norm, "/powershell/6/") {
		return "PowerShell 6"
	}
	return "Windows PowerShell"
}
