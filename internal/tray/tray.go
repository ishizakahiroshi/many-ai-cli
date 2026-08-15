// Package tray はタスクトレイ常駐から Hub の起動・停止・表示を行う。
//
// 解こうとしている問題は 1 つだけで、Windows のデスクトップに起動用と停止用の
// ショートカットが 2 個並んでいること（`Many AI Hub Start.lnk` /
// `Many AI Hub Stop.lnk`）。トレイならアイコン 1 個で両方を賄えて、しかも
// 画面の枠を取らない。
//
// 窓は作らない。UI は今までどおり既定ブラウザのタブで開く
// （CLAUDE.md「デスクトップアプリ化・Microsoft Store 提出は提案しない」）。
// OS のネイティブ通知も出さない。承認待ちの知らせは Web UI 側の 3 点
// （タイトル点滅・favicon の件数バッジ・通知音）で成立している。
//
// 常駐先は serve ではなく独立プロセス（`many-ai-cli tray`）にした。plan は
// 「serve 自身が持つ方が単純」と書いているが、それだと Hub を停止した時点で
// トレイも消えるため、メニューの「Hub を停止」と「終了」が同じ操作になり、
// 「止まっている状態から起動して開く」もできない。トレイが Hub より長生き
// する必要がある（docs/local/plan_tray-resident-hub-lifecycle.md C1 の
// 「常駐プロセスをどこに置くかを決める」に対する判断）。
package tray

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/hub"
)

// ErrUnsupported は Windows 以外で Run を呼んだときに返る。
var ErrUnsupported = errors.New("tray is only available on Windows")

// hubStartTimeout は serve を起動してから URL が引けるようになるまで待つ上限。
// 待てなかった場合はブラウザを開かずエラーにする（空タブを開かない）。
const hubStartTimeout = 20 * time.Second

// hubPollInterval は起動待ちのポーリング間隔。
const hubPollInterval = 250 * time.Millisecond

// Run はトレイ常駐を開始し、終了が選ばれるまで戻らない。
func Run(cfg *config.Config) error { return run(cfg) }

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
// Hub が道連れにならないよう、プロセスグループを分けてコンソールも出さない
// （detach の OS 依存部分は tray_windows.go 側の detachSysProcAttr が持つ）。
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
	// 起動した子は待たない（常駐させる）。Wait を呼ばないとゾンビが残る
	// プラットフォームがあるので、終了だけ別ゴルーチンで回収しておく。
	go func() { _ = cmd.Wait() }()
	return nil
}

// waitForHubURL は resolve が URL を返すまで poll 間隔で待つ。
// 期限内に返らなければエラーにする（URL 無しでブラウザを開いて空タブを出さない）。
// resolve を引数に取るのは、Hub を立てずに待ち挙動を試せるようにするため。
func waitForHubURL(resolve func() (string, bool), timeout, poll time.Duration) (string, error) {
	deadline := time.Now().Add(timeout)
	for {
		if url, ok := resolve(); ok {
			return url, nil
		}
		if time.Now().After(deadline) {
			return "", fmt.Errorf("hub did not become reachable within %s", timeout)
		}
		time.Sleep(poll)
	}
}
