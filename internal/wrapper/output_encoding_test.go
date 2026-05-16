package wrapper

import "testing"

func TestRepairMojibakeUTF8Japanese(t *testing.T) {
	in := []byte("ãƒ­ãƒ¼ã‚«ãƒ«ãƒ‡ã‚£ãƒ¬ã‚¯ãƒˆãƒªã® MD ãƒ•ã‚¡ã‚¤ãƒ«ä¸€è¦§å–å¾—")
	got := string(repairMojibakeUTF8(in))
	want := "ローカルディレクトリの MD ファイル一覧取得"
	if got != want {
		t.Fatalf("want %q, got %q", want, got)
	}
}

func TestRepairMojibakeUTF8BoxDrawing(t *testing.T) {
	in := []byte("â”€â”€â”€â¯ Â· â†‘")
	got := string(repairMojibakeUTF8(in))
	want := "───❯ · ↑"
	if got != want {
		t.Fatalf("want %q, got %q", want, got)
	}
}

func TestRepairMojibakeUTF8LeavesValidJapanese(t *testing.T) {
	in := []byte("ローカルディレクトリの MD ファイル一覧取得")
	got := repairMojibakeUTF8(in)
	if string(got) != string(in) {
		t.Fatalf("valid UTF-8 should pass through: %q", string(got))
	}
}

func TestRepairMojibakeUTF8LeavesPlainASCII(t *testing.T) {
	in := []byte("WARNING: approval patterns fetch failed")
	got := repairMojibakeUTF8(in)
	if string(got) != string(in) {
		t.Fatalf("plain ASCII should pass through: %q", string(got))
	}
}
