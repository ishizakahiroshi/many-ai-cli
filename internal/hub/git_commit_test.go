package hub

import (
	"strings"
	"testing"
)

// TestExtractCommitMarker は AI 出力からマーカー対を抽出できること、特に注入プロンプトの
// エコー（同じマーカー語を含む）を取り違えず、最後の対を採ることを検証する（方針3 / 方式1）。
func TestExtractCommitMarker(t *testing.T) {
	t.Run("基本抽出", func(t *testing.T) {
		buf := "noise\n" + commitMsgMarkerOpen + "\nfeat: add thing\n\nbody line 1\nbody line 2\n" + commitMsgMarkerClose + "\ntrailing"
		sub, body, ok := extractCommitMarker(buf)
		if !ok || sub != "feat: add thing" || body != "body line 1\nbody line 2" {
			t.Fatalf("got ok=%v sub=%q body=%q", ok, sub, body)
		}
	})

	t.Run("プロンプトエコーを取り違えない", func(t *testing.T) {
		echo := aiCommitPrompt(true) // open/close マーカー語を本文に含む
		buf := echo + "\n" + commitMsgMarkerOpen + "\nfix: real subject\n" + commitMsgMarkerClose + "\n"
		sub, body, ok := extractCommitMarker(buf)
		if !ok || sub != "fix: real subject" || body != "" {
			t.Fatalf("got ok=%v sub=%q body=%q", ok, sub, body)
		}
	})

	t.Run("エコー単独窓では未確定（AI本応答前の中間状態）", func(t *testing.T) {
		// AI の本応答が届く前は、バッファに注入プロンプトのエコーしか無い。
		// マーカー語は文中インラインなので "マーカー単独行" 一致では拾わず ok=false になり、
		// プロンプト指示文断片を subject に取り違えない。
		for _, ja := range []bool{true, false} {
			if _, _, ok := extractCommitMarker(aiCommitPrompt(ja)); ok {
				t.Fatalf("echo-only buffer (ja=%v) must not yield a marker pair", ja)
			}
		}
		// その後 AI の本応答が届けば正しく抽出できる。
		buf := aiCommitPrompt(true) + "\n" + commitMsgMarkerOpen + "\nfeat: done\n" + commitMsgMarkerClose
		if sub, _, ok := extractCommitMarker(buf); !ok || sub != "feat: done" {
			t.Fatalf("after real response: got ok=%v sub=%q", ok, sub)
		}
	})

	t.Run("TUI 罫線ガターを除去", func(t *testing.T) {
		buf := commitMsgMarkerOpen + "\n│ refactor: tidy up\n│ \n│ details here\n" + commitMsgMarkerClose
		sub, body, ok := extractCommitMarker(buf)
		if !ok || sub != "refactor: tidy up" || !strings.Contains(body, "details here") {
			t.Fatalf("got ok=%v sub=%q body=%q", ok, sub, body)
		}
	})

	t.Run("close 未到達なら未確定", func(t *testing.T) {
		buf := commitMsgMarkerOpen + "\nfeat: partial"
		if _, _, ok := extractCommitMarker(buf); ok {
			t.Fatalf("expected ok=false while close marker is absent")
		}
	})

	t.Run("OPEN マーカーに subject が連結（Claude TUI 再描画の実再現）", func(t *testing.T) {
		// Claude Code の TUI 再描画 + StripANSI で、行頭バレット直後に OPEN マーカーと
		// subject が同一行へ連結される（実セッションログで確認した壊れ方）。
		buf := "noise\n●" + commitMsgMarkerOpen + "docs: add guide\n\nbody line 1\nbody line 2\n  " + commitMsgMarkerClose + "\ntrailing"
		sub, body, ok := extractCommitMarker(buf)
		if !ok || sub != "docs: add guide" || body != "body line 1\nbody line 2" {
			t.Fatalf("got ok=%v sub=%q body=%q", ok, sub, body)
		}
	})

	t.Run("CLOSE マーカーが本文末尾に連結", func(t *testing.T) {
		buf := commitMsgMarkerOpen + "\nfix: x\nbody" + commitMsgMarkerClose
		sub, body, ok := extractCommitMarker(buf)
		if !ok || sub != "fix: x" || body != "body" {
			t.Fatalf("got ok=%v sub=%q body=%q", ok, sub, body)
		}
	})

	t.Run("連結 OPEN でもプロンプトエコーを取り違えない", func(t *testing.T) {
		// エコー（マーカー語は文中インライン）の後に、連結 OPEN 形式の本応答が届くケース。
		buf := aiCommitPrompt(true) + "\n●" + commitMsgMarkerOpen + "feat: real\n" + commitMsgMarkerClose
		sub, _, ok := extractCommitMarker(buf)
		if !ok || sub != "feat: real" {
			t.Fatalf("got ok=%v sub=%q", ok, sub)
		}
	})
}

func sf(status, path string) gitStatusFile {
	return gitStatusFile{Status: status, Path: path}
}

// TestSuggestCommitMessagePrefix は変更種別から conventional commit prefix が
// 正しく出し分けられること（方針1）を検証する。
func TestSuggestCommitMessagePrefix(t *testing.T) {
	cases := []struct {
		name       string
		files      []gitStatusFile
		diff       string
		wantPrefix string
	}{
		{
			name:       "新規ファイルのみは feat",
			files:      []gitStatusFile{sf("A", "internal/hub/new_feature.go")},
			wantPrefix: "feat:",
		},
		{
			name:       "新規関数追加は feat",
			files:      []gitStatusFile{sf("M", "internal/hub/server.go")},
			diff:       "+++ b/internal/hub/server.go\n+func DoNewThing() {\n",
			wantPrefix: "feat:",
		},
		{
			name:       "既存コードの編集だけは refactor（feat にしない）",
			files:      []gitStatusFile{sf("M", "internal/hub/server.go")},
			diff:       "+++ b/internal/hub/server.go\n+\tx := 1\n-\tx := 2\n",
			wantPrefix: "refactor:",
		},
		{
			name:       "依存ファイルのみは chore(deps)",
			files:      []gitStatusFile{sf("M", "go.mod"), sf("M", "go.sum")},
			wantPrefix: "chore(deps):",
		},
		{
			name:       "ドキュメントのみは docs",
			files:      []gitStatusFile{sf("M", "docs/guide.md")},
			wantPrefix: "docs:",
		},
		{
			name:       "テストのみは test",
			files:      []gitStatusFile{sf("M", "internal/hub/server_test.go")},
			wantPrefix: "test:",
		},
		{
			name:       "CSS のみは style",
			files:      []gitStatusFile{sf("M", "web/src/styles/chat.css")},
			wantPrefix: "style:",
		},
		{
			name:       "コード削除のみは refactor",
			files:      []gitStatusFile{sf("D", "internal/hub/old.go")},
			wantPrefix: "refactor:",
		},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			subject, _ := suggestCommitMessage(c.files, "", c.diff, "", "ja")
			if got := subject; len(got) < len(c.wantPrefix) || got[:len(c.wantPrefix)] != c.wantPrefix {
				t.Errorf("prefix mismatch: got %q, want prefix %q", got, c.wantPrefix)
			}
		})
	}
}

// TestSuggestCommitMessageDefaultSubject は無情報な「scope を更新」を避け、
// 代表的な変更ファイル名で要約されること（方針2）を検証する。
func TestSuggestCommitMessageDefaultSubject(t *testing.T) {
	files := []gitStatusFile{
		sf("M", "web/src/app.ts"),
		sf("M", "web/src/app/state.ts"),
		sf("M", "web/src/styles.css"),
	}
	subjectJa, _ := suggestCommitMessage(files, "", "", "", "ja")
	if want := "refactor: app.ts ほか 2 件 を変更"; subjectJa != want {
		t.Errorf("ja default subject: got %q, want %q", subjectJa, want)
	}

	subjectEn, _ := suggestCommitMessage(files, "", "", "", "en")
	if want := "refactor: update app.ts (+2 more)"; subjectEn != want {
		t.Errorf("en default subject: got %q, want %q", subjectEn, want)
	}
}
