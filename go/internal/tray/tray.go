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
//
// 常駐に入る導線は setup が作る 2 個のショートカット（デスクトップ +
// スタートアップフォルダ）で、どちらも `many-ai-cli tray` を指す。手で毎回
// 起動しないと出てこないものは常駐と呼べないため、ログイン時の自動起動まで
// setup の仕事に含めてある（v0.7.0 の初版はデスクトップの 1 個だけだった）。
// 自動起動と手動起動が重なると同じアイコンが 2 個並ぶので、名前付き mutex で
// プロセスを 1 つに保ち、2 つ目は Hub を開いて終わる。
//
// Hub とのやり取り（起動・停止・URL 解決）は tray_windows.go に置いてある。
// Windows からしか呼ばれないものを本ファイルへ置くと、他 OS のビルドで
// 未使用になり staticcheck が落ちる。ここには全 OS で意味のあるものだけ置く。
package tray

import (
	"errors"
	"fmt"
	"time"

	"many-ai-cli/internal/config"
)

// ErrUnsupported は Windows 以外で Run を呼んだときに返る。
var ErrUnsupported = errors.New("tray is only available on Windows")

// Run はトレイ常駐を開始し、終了が選ばれるまで戻らない。
func Run(cfg *config.Config) error { return run(cfg) }

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
