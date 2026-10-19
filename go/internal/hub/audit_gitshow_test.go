package hub

import "testing"

// TestGitNumstatRestoreRenamePath は brace 圧縮リネーム表記の new path 復元を検証する
// （finding #23: dir/{old => new}/file 形式で `}` が残り行数照合が壊れる回帰）。
func TestGitNumstatRestoreRenamePath(t *testing.T) {
	cases := []struct {
		name string
		in   string
		want string
	}{
		// 単純形（ディレクトリ移動なし）— 従来挙動を保持
		{"simple", "old.go => new.go", "new.go"},
		{"simple_with_dir", "src/old.go => dst/new.go", "dst/new.go"},
		// brace 圧縮形 — prefix/suffix を保持して中間セグメントを置換
		{"brace_mid_dir", "src/{old => new}/file.go", "src/new/file.go"},
		{"brace_leading", "{a => b}/file.go", "b/file.go"},
		{"brace_filename", "dir/{old.go => new.go}", "dir/new.go"},
		{"brace_deep", "a/b/{c => d}/e/f.go", "a/b/d/e/f.go"},
		// 純粋ディレクトリ削除相当（new セグメントが空）→ 連続スラッシュを畳む
		{"brace_empty_new", "src/{old => }/file.go", "src/file.go"},
		// brace を含むがリネームでない（ありえないが安全側）
		{"brace_no_arrow", "src/{weird}/file.go", "src/{weird}/file.go"},
		// 通常パス（リネームでない）
		{"plain", "internal/hub/git_show.go", "internal/hub/git_show.go"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := gitNumstatRestoreRenamePath(c.in); got != c.want {
				t.Errorf("gitNumstatRestoreRenamePath(%q) = %q, want %q", c.in, got, c.want)
			}
		})
	}
}

// TestParseNumstatRaw_BraceMidDir は parseNumstatRaw が brace 圧縮形を
// new フルパスでキー化することを確認する（applyNumstat の name-status 側キーと一致させる）。
func TestParseNumstatRaw_BraceMidDir(t *testing.T) {
	raw := "7\t3\tinternal/{old.go => new.go}\n"
	m := parseNumstatRaw(raw)
	if _, bad := m["new.go}"]; bad {
		t.Error("broken key new.go} must not exist")
	}
	e, ok := m["internal/new.go"]
	if !ok {
		t.Fatalf("internal/new.go not found; keys=%v", mapKeys(m))
	}
	if e.added != 7 || e.removed != 3 {
		t.Errorf("internal/new.go: %+v", e)
	}
}

// TestApplyNumstat_BraceRename は brace 圧縮リネームの行数が
// parseNameStatus 側の new フルパス (files[i].Path) と照合して埋まることを確認する。
func TestApplyNumstat_BraceRename(t *testing.T) {
	// name-status は new フルパスを持つ（parseNameStatus が fields[2] を採用）
	files := []gitShowFile{{Status: "R", Path: "internal/new.go"}}
	// numstat は brace 圧縮形で出力される
	raw := "9\t4\tinternal/{old.go => new.go}\n"
	applyNumstat(files, raw)
	if files[0].Added != 9 || files[0].Removed != 4 {
		t.Errorf("brace rename numstat not applied: added=%d removed=%d", files[0].Added, files[0].Removed)
	}
}
