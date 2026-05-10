//go:build windows

package wrapper

import (
	"syscall"
	"unsafe"
)

var (
	kernel32         = syscall.MustLoadDLL("kernel32.dll")
	user32           = syscall.MustLoadDLL("user32.dll")
	setConsoleTitleW = kernel32.MustFindProc("SetConsoleTitleW")
	getConsoleWindow = kernel32.MustFindProc("GetConsoleWindow")
	getModuleHandleW = kernel32.MustFindProc("GetModuleHandleW")
	loadImageW       = user32.MustFindProc("LoadImageW")
	sendMessageW     = user32.MustFindProc("SendMessageW")
)

const (
	wmSetIcon     = 0x0080
	iconSmall     = 0
	iconBig       = 1
	imageIcon     = 1
	lrDefaultSize = 0x40
	lrShared      = 0x8000
)

func setConsoleTitle(title string) {
	p, _ := syscall.UTF16PtrFromString(title)
	_, _, _ = setConsoleTitleW.Call(uintptr(unsafe.Pointer(p)))
}

func setConsoleIcon() {
	hWnd, _, _ := getConsoleWindow.Call()
	if hWnd == 0 {
		return
	}
	hMod, _, _ := getModuleHandleW.Call(0)
	hIcon, _, _ := loadImageW.Call(hMod, 1, imageIcon, 0, 0, lrDefaultSize|lrShared)
	if hIcon == 0 {
		return
	}
	_, _, _ = sendMessageW.Call(hWnd, wmSetIcon, iconSmall, hIcon)
	_, _, _ = sendMessageW.Call(hWnd, wmSetIcon, iconBig, hIcon)
}
