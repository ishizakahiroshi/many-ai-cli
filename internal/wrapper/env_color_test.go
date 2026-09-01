package wrapper

import (
	"slices"
	"testing"
)

func TestEnvWithoutColorSuppressorsDropsNoColor(t *testing.T) {
	in := []string{"PATH=/usr/bin", "NO_COLOR=1", "HOME=/home/box"}
	got := envWithoutColorSuppressors(in)
	if slices.Contains(got, "NO_COLOR=1") {
		t.Fatalf("NO_COLOR should be dropped, got %v", got)
	}
	if len(got) != 2 || got[0] != "PATH=/usr/bin" || got[1] != "HOME=/home/box" {
		t.Fatalf("other entries must survive in order, got %v", got)
	}
}

// 空文字も落とす。「変数が存在するか」だけを見る色判定に引っかからないようにするため。
func TestEnvWithoutColorSuppressorsDropsEmptyNoColor(t *testing.T) {
	got := envWithoutColorSuppressors([]string{"NO_COLOR=", "TERM=dumb"})
	if len(got) != 1 || got[0] != "TERM=dumb" {
		t.Fatalf("got %v", got)
	}
}

// Windows の環境変数名は case-insensitive。
func TestEnvWithoutColorSuppressorsIgnoresCase(t *testing.T) {
	got := envWithoutColorSuppressors([]string{"No_Color=1", "PATH=/usr/bin"})
	if len(got) != 1 || got[0] != "PATH=/usr/bin" {
		t.Fatalf("got %v", got)
	}
}

// FORCE_COLOR / CLICOLOR_FORCE は落とさない（この関数の後ろで明示的に上書きするため、
// ここで消すと「利用者が立てた値」と「こちらが立てる値」の区別が付かなくなる）。
func TestEnvWithoutColorSuppressorsKeepsForceColor(t *testing.T) {
	in := []string{"FORCE_COLOR=0", "CLICOLOR_FORCE=0"}
	got := envWithoutColorSuppressors(in)
	if !slices.Equal(got, in) {
		t.Fatalf("got %v, want %v", got, in)
	}
}

func TestEnvWithoutColorSuppressorsKeepsMalformedEntries(t *testing.T) {
	in := []string{"NOEQUALS", "NO_COLORS=1", "XNO_COLOR=1"}
	got := envWithoutColorSuppressors(in)
	if !slices.Equal(got, in) {
		t.Fatalf("前方一致・部分一致で誤って落としてはいけない: got %v", got)
	}
}

// hub.force_color: true（既定）は TERM/COLORTERM に加えて色を強制し NO_COLOR を落とす。
func TestChildEnvForceColorOn(t *testing.T) {
	got := childEnv([]string{"PATH=/usr/bin", "NO_COLOR=1", "FORCE_COLOR=0"}, true)
	if slices.Contains(got, "NO_COLOR=1") {
		t.Fatalf("NO_COLOR は落とすこと: %v", got)
	}
	for _, want := range []string{"TERM=xterm-256color", "COLORTERM=truecolor", "FORCE_COLOR=3", "CLICOLOR_FORCE=1", "MANY_AI_CLI=1"} {
		if !slices.Contains(got, want) {
			t.Fatalf("%s が無い: %v", want, got)
		}
	}
	// 継承した FORCE_COLOR=0 は後ろの FORCE_COLOR=3 が勝つ（os/exec は最後の値を採用）。
	if slices.Index(got, "FORCE_COLOR=0") > slices.Index(got, "FORCE_COLOR=3") {
		t.Fatalf("上書きが先に来てはいけない: %v", got)
	}
}

// hub.force_color: false は「色を出すな」という利用者の指定を尊重する。
// TERM/COLORTERM の上書き（端末の能力の訂正）だけは従来どおり残す。
func TestChildEnvForceColorOff(t *testing.T) {
	got := childEnv([]string{"PATH=/usr/bin", "NO_COLOR=1"}, false)
	if !slices.Contains(got, "NO_COLOR=1") {
		t.Fatalf("NO_COLOR は残すこと: %v", got)
	}
	for _, ng := range []string{"FORCE_COLOR=3", "CLICOLOR_FORCE=1"} {
		if slices.Contains(got, ng) {
			t.Fatalf("%s を付けてはいけない: %v", ng, got)
		}
	}
	for _, want := range []string{"TERM=xterm-256color", "COLORTERM=truecolor", "MANY_AI_CLI=1"} {
		if !slices.Contains(got, want) {
			t.Fatalf("%s が無い: %v", want, got)
		}
	}
}

// 呼び出し元の os.Environ() を壊さない（append の共有バッキング配列事故の防止）。
func TestChildEnvDoesNotMutateInput(t *testing.T) {
	base := []string{"PATH=/usr/bin", "HOME=/home/box"}
	snapshot := slices.Clone(base)
	_ = childEnv(base, false)
	_ = childEnv(base, true)
	if !slices.Equal(base, snapshot) {
		t.Fatalf("入力を書き換えている: %v", base)
	}
}
