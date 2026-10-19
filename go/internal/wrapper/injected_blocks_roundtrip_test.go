package wrapper

import (
	"os"
	"path/filepath"
	"testing"
)

// 利用者のファイルへ一時的に差し込むブロックは、差し込みと取り除きを何度繰り返しても
// 元のバイト列へ戻らなければならない。既存テストは TrimSpace や Contains で比べていたため、
// 往復のたびに改行が 1 つずつ残る積み上がり（claude の import 行で実際に起きた。
// docs/local/bugfix_claude-import-removal-leaves-blank-lines_2026-09-29.md）を検出できなかった。
// ここでは Hub と同じ順（injectApprovalTargets / removeApprovalTargets）で往復させ、バイト一致で見る。
func TestInjectedBlocksRoundTripRestoresOriginalBytes(t *testing.T) {
	type step func(path string) error
	kinds := []struct {
		name   string
		inject []step
		remove []step
	}{
		{
			name:   "shared block",
			inject: []step{func(p string) error { return InjectRules("codex", p) }},
			remove: []step{func(p string) error { return RemoveRules("codex", p) }},
		},
		{
			name:   "delegation block",
			inject: []step{func(p string) error { return InjectDelegation("codex", p) }},
			remove: []step{RemoveDelegation},
		},
		{
			name: "shared and delegation blocks in hub order",
			inject: []step{
				func(p string) error { return InjectRules("codex", p) },
				func(p string) error { return InjectDelegation("codex", p) },
			},
			remove: []step{
				func(p string) error { return RemoveRules("codex", p) },
				RemoveDelegation,
			},
		},
	}
	originals := []struct {
		name    string
		content string
	}{
		{name: "ends with newline", content: "# rules\n\nbody\n"},
		{name: "no trailing newline", content: "# rules\n\nbody"},
	}
	for _, k := range kinds {
		for _, o := range originals {
			t.Run(k.name+"/"+o.name, func(t *testing.T) {
				withTempHome(t)
				path := filepath.Join(t.TempDir(), "AGENTS.md")
				if err := os.WriteFile(path, []byte(o.content), 0o644); err != nil {
					t.Fatal(err)
				}
				for i := 0; i < 3; i++ {
					for _, inject := range k.inject {
						if err := inject(path); err != nil {
							t.Fatalf("inject #%d failed: %v", i+1, err)
						}
					}
					for _, remove := range k.remove {
						if err := remove(path); err != nil {
							t.Fatalf("remove #%d failed: %v", i+1, err)
						}
					}
				}
				data, err := os.ReadFile(path)
				if err != nil {
					t.Fatal(err)
				}
				if string(data) != o.content {
					t.Fatalf("3 inject/remove round trips changed the file:\nwant: %q\ngot:  %q", o.content, string(data))
				}
			})
		}
	}
}
