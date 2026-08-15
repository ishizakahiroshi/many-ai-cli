package tray

import (
	"errors"
	"testing"
	"time"
)

func TestWaitForHubURLReturnsAsSoonAsResolved(t *testing.T) {
	calls := 0
	url, err := waitForHubURL(func() (string, bool) {
		calls++
		// 起動直後は URL を引けないのが普通なので、数回空振りしてから返す。
		if calls < 3 {
			return "", false
		}
		return "http://127.0.0.1:47777/?token=abc", true
	}, time.Second, time.Millisecond)
	if err != nil {
		t.Fatalf("waitForHubURL: %v", err)
	}
	if url != "http://127.0.0.1:47777/?token=abc" {
		t.Fatalf("url = %q", url)
	}
	if calls != 3 {
		t.Fatalf("resolve の呼び出し回数 = %d, want 3（解決した時点で止まること）", calls)
	}
}

// 期限切れで URL を返さないことを固定する。空文字を返してしまうと、
// 呼び出し側がブラウザで空タブを開く。
func TestWaitForHubURLTimesOutWithoutURL(t *testing.T) {
	url, err := waitForHubURL(func() (string, bool) { return "", false }, 10*time.Millisecond, time.Millisecond)
	if err == nil {
		t.Fatal("期限切れなのに err が nil")
	}
	if url != "" {
		t.Fatalf("期限切れなのに url = %q", url)
	}
}

// 既に起動していれば 1 回目で返る（起動待ちのスリープを挟まない）。
func TestWaitForHubURLDoesNotSleepWhenAlreadyUp(t *testing.T) {
	start := time.Now()
	if _, err := waitForHubURL(func() (string, bool) { return "http://127.0.0.1:1/?token=x", true },
		time.Second, 500*time.Millisecond); err != nil {
		t.Fatalf("waitForHubURL: %v", err)
	}
	if elapsed := time.Since(start); elapsed > 200*time.Millisecond {
		t.Fatalf("即座に返らなかった: %s", elapsed)
	}
}

func TestErrUnsupportedIsComparable(t *testing.T) {
	// 呼び出し側が errors.Is で分岐できること（Windows 以外での案内文に使う）。
	if !errors.Is(ErrUnsupported, ErrUnsupported) {
		t.Fatal("ErrUnsupported が errors.Is で判定できない")
	}
}
