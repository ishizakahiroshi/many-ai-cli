//go:build windows

package tray

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"runtime"
	"syscall"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/hub"
)

// Win32 の呼び出しはすべて NewLazySystemDLL + NewProc 経由にしてある。cgo は
// 使わない。goreleaser は全ターゲットで CGO_ENABLED=0 を前提にしており
// （SQLite に modernc を選んでいるのも同じ理由）、ここを崩すとリリース
// パイプライン全体に波及する。同じ書き方の先例が
// internal/launcher/console_windows.go にある。
var (
	user32   = windows.NewLazySystemDLL("user32.dll")
	shell32  = windows.NewLazySystemDLL("shell32.dll")
	kernel32 = windows.NewLazySystemDLL("kernel32.dll")

	procRegisterClassExW = user32.NewProc("RegisterClassExW")
	procCreateWindowExW  = user32.NewProc("CreateWindowExW")
	procDefWindowProcW   = user32.NewProc("DefWindowProcW")
	procDestroyWindow    = user32.NewProc("DestroyWindow")
	procGetMessageW      = user32.NewProc("GetMessageW")
	procTranslateMessage = user32.NewProc("TranslateMessage")
	procDispatchMessageW = user32.NewProc("DispatchMessageW")
	procPostQuitMessage  = user32.NewProc("PostQuitMessage")
	procCreatePopupMenu  = user32.NewProc("CreatePopupMenu")
	procAppendMenuW      = user32.NewProc("AppendMenuW")
	procDestroyMenu      = user32.NewProc("DestroyMenu")
	procTrackPopupMenu   = user32.NewProc("TrackPopupMenu")
	procSetForegroundWnd = user32.NewProc("SetForegroundWindow")
	procGetCursorPos     = user32.NewProc("GetCursorPos")
	procLoadIconW        = user32.NewProc("LoadIconW")
	procLoadImageW       = user32.NewProc("LoadImageW")
	procDestroyIcon      = user32.NewProc("DestroyIcon")
	procGetSystemMetrics = user32.NewProc("GetSystemMetrics")
	procPostMessageW     = user32.NewProc("PostMessageW")
	procShellNotifyIconW = shell32.NewProc("Shell_NotifyIconW")
	procGetModuleHandleW = kernel32.NewProc("GetModuleHandleW")
	procFreeConsole      = kernel32.NewProc("FreeConsole")
	procCreateMutexW     = kernel32.NewProc("CreateMutexW")
)

// hubStartTimeout は serve を起動してから URL が引けるようになるまで待つ上限。
// 待てなかった場合はブラウザを開かずエラーにする（空タブを開かない）。
const hubStartTimeout = 20 * time.Second

// hubPollInterval は起動待ちのポーリング間隔。
const hubPollInterval = 250 * time.Millisecond

// openHub は Hub が動いていなければ起動し、既定ブラウザで開く。
// 動いていればそのまま開く。
func openHub(cfg *config.Config) error {
	if url, ok := hub.RunningURL(cfg); ok {
		return hub.OpenURL(url)
	}
	if err := startHubDetached(); err != nil {
		return fmt.Errorf("start hub: %w", err)
	}
	url, err := waitForHubURL(func() (string, bool) { return hub.RunningURL(cfg) }, hubStartTimeout, hubPollInterval)
	if err != nil {
		return err
	}
	return hub.OpenURL(url)
}

// stopHub は Hub を停止する。トレイ自身は残す。
func stopHub(cfg *config.Config) error {
	if !hub.IsRunning(cfg) {
		return nil
	}
	return hub.Stop(cfg)
}

// startHubDetached は自分自身の exe を `serve` で起動する。トレイを閉じても
// Hub が道連れにならないよう、プロセスグループを分けてコンソールも出さない。
func startHubDetached() error {
	exe, err := os.Executable()
	if err != nil {
		return fmt.Errorf("locate executable: %w", err)
	}
	cmd := exec.Command(exe, "serve") // #nosec G204 -- 引数は固定文字列、パスは os.Executable の自プロセス
	cmd.SysProcAttr = detachSysProcAttr()
	if err := cmd.Start(); err != nil {
		return err
	}
	// 起動した子は待たない（常駐させる）。終了だけ別ゴルーチンで回収しておく。
	go func() { _ = cmd.Wait() }()
	return nil
}

const (
	wmDestroy      = 0x0002
	wmClose        = 0x0010
	wmCommand      = 0x0111
	wmRButtonUp    = 0x0205
	wmLButtonUp    = 0x0202
	wmNull         = 0x0000
	wmTrayCallback = 0x0400 + 1 // WM_USER + 1（トレイのコールバックに使う慣例値）

	nimAdd    = 0x0000
	nimModify = 0x0001
	nimDelete = 0x0002

	nifMessage = 0x0001
	nifIcon    = 0x0002
	nifTip     = 0x0004

	mfString    = 0x0000
	mfSeparator = 0x0800
	mfGrayed    = 0x0001

	tpmLeftAlign   = 0x0000
	tpmRightButton = 0x0002

	idiApplication = 32512

	imageIcon      = 0x0001 // IMAGE_ICON（0 は IMAGE_BITMAP なので取り違えない）
	lrDefaultColor = 0x0000
	smCXSmIcon     = 49
	smCYSmIcon     = 50

	// appIconResourceID は winres/winres.json の RT_GROUP_ICON "#1"。
	// go-winres が exe へ焼く many-ai-cli 本体のアイコン。
	appIconResourceID = 1

	// メニュー項目 ID。0 は TrackPopupMenu の「選ばれなかった」と区別が付かないので使わない。
	idOpenHub = 1
	idStopHub = 2
	idQuit    = 3
)

// trayMutexName はトレイの二重起動を防ぐための名前付き mutex。
// Local\ 名前空間なのでログオンセッションごとに 1 つ。
const trayMutexName = `Local\ManyAICLITray`

type wndClassExW struct {
	cbSize        uint32
	style         uint32
	lpfnWndProc   uintptr
	cbClsExtra    int32
	cbWndExtra    int32
	hInstance     windows.Handle
	hIcon         windows.Handle
	hCursor       windows.Handle
	hbrBackground windows.Handle
	lpszMenuName  *uint16
	lpszClassName *uint16
	hIconSm       windows.Handle
}

type point struct{ X, Y int32 }

type msg struct {
	hwnd    windows.Handle
	message uint32
	wParam  uintptr
	lParam  uintptr
	time    uint32
	pt      point
}

// notifyIconDataW は NOTIFYICONDATAW。szTip は 128 UTF-16 要素の固定長。
type notifyIconDataW struct {
	cbSize           uint32
	hWnd             windows.Handle
	uID              uint32
	uFlags           uint32
	uCallbackMessage uint32
	hIcon            windows.Handle
	szTip            [128]uint16
}

// trayState はメッセージループとメニュー操作の間で共有する最小限の状態。
// WndProc は syscall.NewCallback 経由で呼ばれるため引数でこれを渡せない。
// トレイはプロセスに 1 つしか作らないのでパッケージ変数で保持する。
var trayState struct {
	cfg  *config.Config
	nid  notifyIconDataW
	hwnd windows.Handle
}

func run(cfg *config.Config) error {
	// Win32 windows and their message queue are thread-affine: RegisterClass,
	// CreateWindow and the GetMessage loop below must all run on the same OS
	// thread. Without pinning, the goroutine can migrate threads and the tray
	// icon stops responding to menu clicks.
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	trayState.cfg = cfg

	// many-ai-cli はコンソールアプリなので、ショートカットから起動すると
	// コンソール窓が付いてくる。トレイに窓は要らないので切り離す。
	// 既に窓が無ければ失敗するが、それは想定内なので戻り値を見ない。
	_, _, _ = procFreeConsole.Call()

	if !acquireNamedMutex(trayMutexName) {
		// 既にトレイが常駐している。ログイン時の自動起動が入ったので、そこへ
		// デスクトップアイコンのダブルクリックが重なるのが普通の使い方になった。
		// アイコンを 2 個並べても押した人には区別が付かないので増やさない。
		// 窓もコンソールも無いのでエラーを見せる先が無く、黙って終わるより
		// 「アイコンを押した人が望んでいるであろうこと」をして終わる方が良い。
		return openHub(cfg)
	}

	hInstance, _, _ := procGetModuleHandleW.Call(0)
	className, err := windows.UTF16PtrFromString("ManyAICLITrayWindow")
	if err != nil {
		return fmt.Errorf("class name: %w", err)
	}

	wc := wndClassExW{
		cbSize:        uint32(unsafe.Sizeof(wndClassExW{})),
		lpfnWndProc:   syscall.NewCallback(wndProc),
		hInstance:     windows.Handle(hInstance),
		lpszClassName: className,
	}
	if atom, _, callErr := procRegisterClassExW.Call(uintptr(unsafe.Pointer(&wc))); atom == 0 {
		return fmt.Errorf("RegisterClassEx: %w", callErr)
	}

	// メッセージ受信専用の隠しウィンドウ。画面には出さない（窓は作らない方針）。
	hwnd, _, callErr := procCreateWindowExW.Call(
		0,
		uintptr(unsafe.Pointer(className)),
		uintptr(unsafe.Pointer(className)),
		0, 0, 0, 0, 0,
		0, 0, hInstance, 0,
	)
	if hwnd == 0 {
		return fmt.Errorf("CreateWindowEx: %w", callErr)
	}
	trayState.hwnd = windows.Handle(hwnd)

	hIcon, ownIcon := loadTrayIcon(hInstance)
	if ownIcon {
		// NIM_DELETE より後に走らせる（defer は LIFO なのでここで登録すると後になる）。
		// 表示中のアイコンのハンドルを先に壊さないため。
		defer func() { _, _, _ = procDestroyIcon.Call(uintptr(hIcon)) }()
	}

	trayState.nid = notifyIconDataW{
		cbSize:           uint32(unsafe.Sizeof(notifyIconDataW{})),
		hWnd:             trayState.hwnd,
		uID:              1,
		uFlags:           nifMessage | nifIcon | nifTip,
		uCallbackMessage: wmTrayCallback,
		hIcon:            hIcon,
	}
	copyTip(&trayState.nid, "many-ai-cli")

	if ok, _, callErr := procShellNotifyIconW.Call(nimAdd, uintptr(unsafe.Pointer(&trayState.nid))); ok == 0 {
		_, _, _ = procDestroyWindow.Call(hwnd)
		return fmt.Errorf("Shell_NotifyIcon(NIM_ADD): %w", callErr)
	}
	// 終了経路がどこを通っても必ずアイコンを消す。消し損ねるとプロセスが
	// 消えた後もゴーストアイコンが残り、マウスを乗せるまで消えない。
	defer func() { _, _, _ = procShellNotifyIconW.Call(nimDelete, uintptr(unsafe.Pointer(&trayState.nid))) }()

	var m msg
	for {
		ret, _, _ := procGetMessageW.Call(uintptr(unsafe.Pointer(&m)), 0, 0, 0)
		// GetMessage は WM_QUIT で 0、エラーで -1 を返す。どちらもループを抜ける。
		if int32(ret) <= 0 {
			return nil
		}
		_, _, _ = procTranslateMessage.Call(uintptr(unsafe.Pointer(&m)))
		_, _, _ = procDispatchMessageW.Call(uintptr(unsafe.Pointer(&m)))
	}
}

// loadTrayIcon はトレイに出すアイコンを返す。第 2 戻り値が true のときは
// 呼び出し側が DestroyIcon する必要がある。
//
// exe に焼いてあるアプリアイコン（winres の RT_GROUP_ICON #1）を、システムの
// 小アイコンサイズで読む。LoadIcon だと大アイコン（32px 相当）が返り、通知領域で
// 縮小されて滲むため、.ico に入っている 16px の絵を使えるこちらを選ぶ。
// go-winres を通していないビルド（go build 直叩き）にはリソースが無く 0 が返るので、
// そのときだけ Windows 既定のアプリアイコンへ落とす。既定アイコンは共有なので破棄しない。
func loadTrayIcon(hInstance uintptr) (windows.Handle, bool) {
	cx, _, _ := procGetSystemMetrics.Call(smCXSmIcon)
	cy, _, _ := procGetSystemMetrics.Call(smCYSmIcon)
	if h, _, _ := procLoadImageW.Call(hInstance, appIconResourceID, imageIcon, cx, cy, lrDefaultColor); h != 0 {
		return windows.Handle(h), true
	}
	h, _, _ := procLoadIconW.Call(0, uintptr(idiApplication))
	return windows.Handle(h), false
}

// acquireNamedMutex は name の mutex を取れたら true を返す。既に誰かが持っていれば
// false（＝別のトレイが常駐している）。
//
// 取れたハンドルは意図的に閉じない。プロセスが生きている間だけ名前を押さえたいので、
// 解放は OS のプロセス終了に任せる。名前を引数に取るのは、テストが実機で常駐中の
// トレイに影響されないようにするため（waitForHubURL が resolve を受け取るのと同じ理由）。
func acquireNamedMutex(name string) bool {
	namePtr, err := windows.UTF16PtrFromString(name)
	if err != nil {
		// 名前が作れないのは異常だが、それを理由に常駐できない方が困るので通す。
		return true
	}
	h, _, callErr := procCreateMutexW.Call(0, 1, uintptr(unsafe.Pointer(namePtr)))
	if h == 0 {
		return true
	}
	if errors.Is(callErr, windows.ERROR_ALREADY_EXISTS) {
		// 既存の mutex を指すハンドルが返っているので閉じる（所有権は取れていない）。
		_ = windows.CloseHandle(windows.Handle(h))
		return false
	}
	return true
}

func copyTip(nid *notifyIconDataW, tip string) {
	utf16Tip, err := windows.UTF16FromString(tip)
	if err != nil {
		return
	}
	if len(utf16Tip) > len(nid.szTip) {
		utf16Tip = utf16Tip[:len(nid.szTip)-1]
		utf16Tip = append(utf16Tip, 0)
	}
	copy(nid.szTip[:], utf16Tip)
}

func wndProc(hwnd windows.Handle, message uint32, wParam, lParam uintptr) uintptr {
	switch message {
	case wmTrayCallback:
		// 左クリックも右クリックも同じメニューを出す。左クリックで即ブラウザを
		// 開く案は、誤爆でタブが増えるので採らない。
		switch uint32(lParam) {
		case wmRButtonUp, wmLButtonUp:
			showMenu(hwnd)
		}
		return 0
	case wmCommand:
		onCommand(uint32(wParam) & 0xffff)
		return 0
	case wmClose:
		_, _, _ = procDestroyWindow.Call(uintptr(hwnd))
		return 0
	case wmDestroy:
		_, _, _ = procPostQuitMessage.Call(0)
		return 0
	}
	ret, _, _ := procDefWindowProcW.Call(uintptr(hwnd), uintptr(message), wParam, lParam)
	return ret
}

func showMenu(hwnd windows.Handle) {
	menu, _, _ := procCreatePopupMenu.Call()
	if menu == 0 {
		return
	}
	defer func() { _, _, _ = procDestroyMenu.Call(menu) }()

	running := hub.IsRunning(trayState.cfg)
	appendMenuItem(menu, mfString, idOpenHub, "Hub を開く")
	stopFlags := uintptr(mfString)
	if !running {
		// 止まっているときに「停止」を押せると、押しても何も起きない項目になる。
		stopFlags |= mfGrayed
	}
	appendMenuItem(menu, stopFlags, idStopHub, "Hub を停止")
	_, _, _ = procAppendMenuW.Call(menu, mfSeparator, 0, 0)
	appendMenuItem(menu, mfString, idQuit, "終了")

	var pt point
	_, _, _ = procGetCursorPos.Call(uintptr(unsafe.Pointer(&pt)))
	// SetForegroundWindow を挟まないと、メニュー外をクリックしても閉じずに
	// 残る（Win32 の既知の作法）。
	_, _, _ = procSetForegroundWnd.Call(uintptr(hwnd))
	_, _, _ = procTrackPopupMenu.Call(
		menu, tpmLeftAlign|tpmRightButton,
		uintptr(pt.X), uintptr(pt.Y), 0, uintptr(hwnd), 0,
	)
	// メニューを閉じた後に WM_NULL を投げるのも同じ作法。これが無いと次の 1 回だけ
	// メニューが反応しないことがある。
	_, _, _ = procPostMessageW.Call(uintptr(hwnd), wmNull, 0, 0)
}

func appendMenuItem(menu uintptr, flags uintptr, id uint32, label string) {
	text, err := windows.UTF16PtrFromString(label)
	if err != nil {
		return
	}
	_, _, _ = procAppendMenuW.Call(menu, flags, uintptr(id), uintptr(unsafe.Pointer(text)))
}

func onCommand(id uint32) {
	switch id {
	case idOpenHub:
		// Hub の起動待ちで最大 20 秒ブロックする。メッセージループを止めると
		// トレイが無反応になるので必ず別ゴルーチンで走らせる。
		go func() { _ = openHub(trayState.cfg) }()
	case idStopHub:
		go func() { _ = stopHub(trayState.cfg) }()
	case idQuit:
		_, _, _ = procDestroyWindow.Call(uintptr(trayState.hwnd))
	}
}

// detachSysProcAttr は serve をトレイから切り離して起動するための設定。
// CREATE_NEW_PROCESS_GROUP でトレイ終了時のシグナルを伝播させず、
// CREATE_NO_WINDOW でコンソール窓を出さない。
func detachSysProcAttr() *syscall.SysProcAttr {
	return &syscall.SysProcAttr{
		CreationFlags: windows.CREATE_NEW_PROCESS_GROUP | windows.CREATE_NO_WINDOW,
		HideWindow:    true,
	}
}
