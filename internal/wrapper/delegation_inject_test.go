package wrapper

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// ファイル注入の対象は「セッション単位の口を持たない provider」だけ。
// claude と grok へ入れると、per-session の経路と二重に渡ることになる。
func TestDelegationBlockTargetsOnlyProvidersWithoutASessionFlag(t *testing.T) {
	for _, p := range []string{"codex", "copilot", "cursor-agent"} {
		if !providerUsesDelegationBlock(p) {
			t.Fatalf("%q はファイル注入の対象のはず", p)
		}
	}
	// claude / grok は per-session（delegation.go の DelegationProviderArgs）で渡す。
	// opencode は読ませる経路が未調査なので対象外。
	for _, p := range []string{"claude", "grok", "opencode", "shell", ""} {
		if providerUsesDelegationBlock(p) {
			t.Fatalf("%q をファイル注入の対象にしている", p)
		}
	}
}

func TestInjectDelegationWritesRemovesAndIsIdempotent(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	target := filepath.Join(t.TempDir(), "AGENTS.md")
	original := "# プロジェクトの指示\n\nここは利用者が書いた本文。\n"
	if err := os.WriteFile(target, []byte(original), 0o644); err != nil {
		t.Fatal(err)
	}

	if err := InjectDelegation("codex", target); err != nil {
		t.Fatalf("InjectDelegation: %v", err)
	}
	after, err := os.ReadFile(target)
	if err != nil {
		t.Fatal(err)
	}
	body := string(after)
	if !strings.HasPrefix(body, original) {
		t.Fatal("利用者が書いた本文を壊している")
	}
	for _, want := range []string{delegationBlockStart, delegationBlockEnd, "orchestrate spawn --role", DelegationResidueNeedle} {
		if !strings.Contains(body, want) {
			t.Fatalf("注入後に %q が無い:\n%s", want, body)
		}
	}
	// 承認ルールのブロックと混ざらないこと（責務を分けた理由そのもの）。
	if strings.Contains(body, sharedBlockStart) {
		t.Fatal("承認ルールのマーカーが混ざっている")
	}

	// 2 回目は何も足さない。
	if err := InjectDelegation("codex", target); err != nil {
		t.Fatalf("2 回目の InjectDelegation: %v", err)
	}
	again, err := os.ReadFile(target)
	if err != nil {
		t.Fatal(err)
	}
	if string(again) != body {
		t.Fatal("同じブロックを二重に注入している")
	}

	// 対象外の provider へは何もしない。
	if err := InjectDelegation("claude", target); err != nil {
		t.Fatalf("claude: %v", err)
	}
	if data, _ := os.ReadFile(target); string(data) != body {
		t.Fatal("対象外の provider で内容が変わった")
	}

	// 外したら利用者の本文だけが残る。
	if err := RemoveDelegation(target); err != nil {
		t.Fatalf("RemoveDelegation: %v", err)
	}
	restored, err := os.ReadFile(target)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(restored), DelegationResidueNeedle) {
		t.Fatalf("ブロックが残っている:\n%s", restored)
	}
	if strings.TrimSpace(string(restored)) != strings.TrimSpace(original) {
		t.Fatalf("外した後に利用者の本文が変わっている:\n%q", restored)
	}

	// 注入していないファイルへ Remove を呼んでも壊さない（回収は条件を付けずに走るため）。
	if err := RemoveDelegation(target); err != nil {
		t.Fatalf("2 回目の RemoveDelegation: %v", err)
	}
	if err := RemoveDelegation(filepath.Join(home, "no-such-file.md")); err != nil {
		t.Fatalf("存在しないファイルで失敗している: %v", err)
	}
}

// version が上がったら古いブロックを入れ替える（積み増さない）。
func TestInjectDelegationReplacesStaleBlock(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	target := filepath.Join(t.TempDir(), "AGENTS.md")
	stale := "本文\n\n" + delegationBlockStart + "\n<!-- version: 0 -->\n古い案内\n" + delegationBlockEnd + "\n"
	if err := os.WriteFile(target, []byte(stale), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := InjectDelegation("codex", target); err != nil {
		t.Fatalf("InjectDelegation: %v", err)
	}
	data, err := os.ReadFile(target)
	if err != nil {
		t.Fatal(err)
	}
	body := string(data)
	if strings.Contains(body, "古い案内") {
		t.Fatalf("古いブロックが残っている:\n%s", body)
	}
	if strings.Count(body, delegationBlockStart) != 1 {
		t.Fatalf("ブロックが %d 個ある（1 個であるべき）:\n%s", strings.Count(body, delegationBlockStart), body)
	}
	if !strings.Contains(body, "<!-- version: "+delegationFileVersion+" -->") {
		t.Fatalf("新しい version が入っていない:\n%s", body)
	}
}
