package wrapper

import (
	"slices"
	"testing"

	"many-ai-cli/internal/config"
)

const (
	envNoColorInherited = "NO_COLOR=1"
	envPath             = "PATH=/usr/bin"
)

// 既定（force）: 色を出せるようにする。継承した NO_COLOR は落とす。
func TestChildEnvForce(t *testing.T) {
	got := childEnv([]string{envPath, envNoColorInherited, "FORCE_COLOR=0"}, config.TerminalColorForce)
	if slices.Contains(got, envNoColorInherited) {
		t.Fatalf("NO_COLOR は落とすこと: %v", got)
	}
	for _, want := range []string{envTerm, envColorterm, envMarker, envForceColor, envClicolorForce} {
		if !slices.Contains(got, want) {
			t.Fatalf("%s が無い: %v", want, got)
		}
	}
	// 継承した FORCE_COLOR=0 より後ろに FORCE_COLOR=3 が来る（os/exec は最後の値を採用）。
	if slices.Index(got, "FORCE_COLOR=0") > slices.Index(got, envForceColor) {
		t.Fatalf("上書きが先に来てはいけない: %v", got)
	}
}

// inherit: 起動元の環境をそのまま渡す。色の指定は足さない。
func TestChildEnvInherit(t *testing.T) {
	got := childEnv([]string{envPath, envNoColorInherited}, config.TerminalColorInherit)
	if !slices.Contains(got, envNoColorInherited) {
		t.Fatalf("NO_COLOR は残すこと: %v", got)
	}
	for _, ng := range []string{envForceColor, envClicolorForce, envForceColorZero} {
		if slices.Contains(got, ng) {
			t.Fatalf("%s を足してはいけない: %v", ng, got)
		}
	}
	// envNoColor は継承値と同じ文字列なので、増えていないことを件数で見る。
	if n := countEnv(got, envNoColorInherited); n != 1 {
		t.Fatalf("NO_COLOR は継承した 1 件のままであること（%d 件）: %v", n, got)
	}
	for _, want := range []string{envTerm, envColorterm, envMarker} {
		if !slices.Contains(got, want) {
			t.Fatalf("%s が無い: %v", want, got)
		}
	}
}

// off: 確実に色を消す。NO_COLOR だけでは FORCE_COLOR を先に見る実装に負けるため、
// 継承した強制指定を外して FORCE_COLOR=0 も明示する。
func TestChildEnvOff(t *testing.T) {
	got := childEnv([]string{envPath, "FORCE_COLOR=3", "CLICOLOR_FORCE=1"}, config.TerminalColorOff)
	if slices.Contains(got, envClicolorForce) {
		t.Fatalf("継承した CLICOLOR_FORCE は落とすこと: %v", got)
	}
	for _, want := range []string{envNoColor, envForceColorZero, envTerm, envColorterm, envMarker} {
		if !slices.Contains(got, want) {
			t.Fatalf("%s が無い: %v", want, got)
		}
	}
	// 継承した FORCE_COLOR=3 より後ろに FORCE_COLOR=0 が来ること。
	if slices.Index(got, envForceColor) > slices.Index(got, envForceColorZero) {
		t.Fatalf("上書きが先に来てはいけない: %v", got)
	}
}

// 端末の能力の訂正（TERM / COLORTERM）は 3 方針すべてで渡す。
func TestChildEnvAlwaysFixesTerm(t *testing.T) {
	for _, mode := range []string{config.TerminalColorForce, config.TerminalColorInherit, config.TerminalColorOff} {
		got := childEnv([]string{"TERM=dumb"}, mode)
		if slices.Index(got, "TERM=dumb") > slices.Index(got, envTerm) {
			t.Fatalf("%s: TERM の上書きが効いていない: %v", mode, got)
		}
	}
}

// 打ち間違い・空は既定（force）へ丸める。起動を止めない。
func TestChildEnvUnknownModeFallsBackToForce(t *testing.T) {
	got := childEnv([]string{envNoColorInherited}, "いろつけて")
	if !slices.Contains(got, envForceColor) || slices.Contains(got, envNoColorInherited) {
		t.Fatalf("未知の値は force 扱いにすること: %v", got)
	}
}

// 呼び出し元の os.Environ() を壊さない（append の共有バッキング配列事故の防止）。
func TestChildEnvDoesNotMutateInput(t *testing.T) {
	base := []string{envPath, "HOME=/home/box"}
	snapshot := slices.Clone(base)
	for _, mode := range []string{config.TerminalColorForce, config.TerminalColorInherit, config.TerminalColorOff} {
		_ = childEnv(base, mode)
	}
	if !slices.Equal(base, snapshot) {
		t.Fatalf("入力を書き換えている: %v", base)
	}
}

func TestEnvWithoutIgnoresCaseAndPartialMatches(t *testing.T) {
	in := []string{"No_Color=1", "NO_COLORS=1", "XNO_COLOR=1", "NOEQUALS", envPath}
	got := envWithout(in, noColorEnvName)
	want := []string{"NO_COLORS=1", "XNO_COLOR=1", "NOEQUALS", envPath}
	if !slices.Equal(got, want) {
		t.Fatalf("got %v, want %v", got, want)
	}
}

func countEnv(env []string, want string) int {
	n := 0
	for _, kv := range env {
		if kv == want {
			n++
		}
	}
	return n
}
