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
