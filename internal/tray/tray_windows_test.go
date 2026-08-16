//go:build windows

package tray

import "testing"

// 2 つ目のトレイを起動させないことを固定する。ログイン時の自動起動が入ったので、
// デスクトップアイコンのダブルクリックと重なるのが普通の使い方になった。
// 名前を実物（trayMutexName）と分けているのは、開発機で本物のトレイが常駐して
// いてもテストが落ちないようにするため。
func TestAcquireNamedMutexBlocksSecondInstance(t *testing.T) {
	name := `Local\ManyAICLITrayTest-` + t.Name()

	if !acquireNamedMutex(name) {
		t.Fatal("1 回目の取得に失敗した")
	}
	if acquireNamedMutex(name) {
		t.Fatal("2 回目も取得できてしまった（トレイが 2 個並ぶ）")
	}
}

// 別の名前なら互いに干渉しないこと（名前の取り違えで常駐できなくなるのを防ぐ）。
func TestAcquireNamedMutexIsPerName(t *testing.T) {
	if !acquireNamedMutex(`Local\ManyAICLITrayTest-A-` + t.Name()) {
		t.Fatal("A の取得に失敗した")
	}
	if !acquireNamedMutex(`Local\ManyAICLITrayTest-B-` + t.Name()) {
		t.Fatal("B の取得に失敗した")
	}
}
