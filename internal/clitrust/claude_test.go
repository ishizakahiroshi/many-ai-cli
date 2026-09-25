package clitrust

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"testing"
	"time"
)

// synthetic fixture only — never the user's real ~/.claude.json (見出しの
// 家の不文律・秘密が同居するファイルの全文出力禁止に従う。値はすべて合成）。
const claudeFixtureWithTwoProjects = `{
  "oauthAccount": {"id": "synthetic-account"},
  "bigNumber": 12345678901234567890,
  "nested": {"a": [1, 2, {"b": true}]},
  "projects": {
    "D:/work/sampleApp": {"hasTrustDialogAccepted": false, "extra": 1},
    "D:/work/SampleApp": {"hasTrustDialogAccepted": true}
  }
}`

func writeClaudeFixture(t *testing.T, body string) string {
	t.Helper()
	dir := t.TempDir()
	configPath := filepath.Join(dir, ".claude.json")
	if err := os.WriteFile(configPath, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	return configPath
}

// decodeWithNumbers 比較用のヘルパー。json.Number を使い、大きな整数を
// float64 経由で丸めずに比較する（テスト側の丸め誤差で誤検知しないため）。
func decodeWithNumbers(t *testing.T, data []byte) map[string]any {
	t.Helper()
	dec := json.NewDecoder(bytes.NewReader(data))
	dec.UseNumber()
	var v map[string]any
	if err := dec.Decode(&v); err != nil {
		t.Fatalf("decode: %v", err)
	}
	return v
}

func assertNoLockOrTempLeftover(t *testing.T, configPath string) {
	t.Helper()
	if _, err := os.Stat(configPath + ".lock"); !os.IsNotExist(err) {
		t.Errorf("lock dir left behind: %s (stat err=%v)", configPath+".lock", err)
	}
	matches, err := filepath.Glob(configPath + ".many-ai-cli-*.tmp")
	if err != nil {
		t.Fatal(err)
	}
	if len(matches) != 0 {
		t.Errorf("temp files left behind: %v", matches)
	}
}

// C2 完了条件: 関係ない項目（大きな整数・入れ子・projects の他のキー・大文字
// 小文字だけ違う 2 つのキー）が、書いたあとも値として同じ。
func TestClaudeGrantPreservesUnrelatedValues(t *testing.T) {
	configPath := writeClaudeFixture(t, claudeFixtureWithTwoProjects)

	result, err := claudeGrant(configPath, claudeKeyPlan("D:/tmp/sample-repo/cases/new"))
	if err != nil {
		t.Fatalf("claudeGrant: %v", err)
	}
	if !result.Written {
		t.Fatalf("Written = false, want true")
	}

	before := decodeWithNumbers(t, []byte(claudeFixtureWithTwoProjects))
	updated, err := os.ReadFile(configPath)
	if err != nil {
		t.Fatal(err)
	}
	after := decodeWithNumbers(t, updated)

	for _, key := range []string{"oauthAccount", "bigNumber", "nested"} {
		if !reflect.DeepEqual(before[key], after[key]) {
			t.Errorf("root key %q changed: before=%#v after=%#v", key, before[key], after[key])
		}
	}

	beforeProjects := before["projects"].(map[string]any)
	afterProjects := after["projects"].(map[string]any)
	for _, key := range []string{"D:/work/sampleApp", "D:/work/SampleApp"} {
		if !reflect.DeepEqual(beforeProjects[key], afterProjects[key]) {
			t.Errorf("projects[%q] changed: before=%#v after=%#v", key, beforeProjects[key], afterProjects[key])
		}
	}
	if len(afterProjects) != len(beforeProjects)+1 {
		t.Errorf("projects has %d entries, want %d (existing 2 + the new one)", len(afterProjects), len(beforeProjects)+1)
	}

	assertNoLockOrTempLeftover(t, configPath)
}

// Claude Code の JSON.stringify は < > & をエスケープしない。encoding/json の
// 既定（\u003c 等）のまま書くと、値は同じでも利用者のファイルの見た目が
// 変わる。入れ子の projects の中の値も同じ扱いになることを確かめる。
func TestClaudeGrantDoesNotHTMLEscape(t *testing.T) {
	configPath := writeClaudeFixture(t, `{"note":"a<b>&c","projects":{"D:/work/x":{"hasTrustDialogAccepted":true,"memo":"<&>"}}}`)

	result, err := claudeGrant(configPath, claudeKeyPlan("D:/work/new"))
	if err != nil {
		t.Fatalf("claudeGrant: %v", err)
	}
	if !result.Written || result.Existing != "" {
		t.Fatalf("result = %+v, want Written=true and Existing empty", result)
	}
	updated := mustReadFile(t, configPath)
	for _, want := range []string{`"a<b>&c"`, `"<&>"`} {
		if !bytes.Contains(updated, []byte(want)) {
			t.Errorf("file does not contain %s verbatim:\n%s", want, updated)
		}
	}
	if bytes.Contains(updated, []byte(`\u003c`)) || bytes.Contains(updated, []byte(`\u0026`)) {
		t.Errorf("file contains HTML-escaped characters:\n%s", updated)
	}
	if bytes.HasSuffix(updated, []byte("\n")) {
		t.Error("file ends with a newline; JSON.stringify output does not")
	}
}

// .claude.json がシンボリックリンクのとき、Claude Code 本体はリンク先へ
// 書く（書き込み関数に allowSymlink を渡している）。こちらもリンクを普通の
// ファイルで置き換えず、リンク先へ書く。
func TestClaudeGrantWritesThroughASymlink(t *testing.T) {
	realDir := t.TempDir()
	realPath := filepath.Join(realDir, "real.claude.json")
	if err := os.WriteFile(realPath, []byte(claudeFixtureWithTwoProjects), 0o600); err != nil {
		t.Fatal(err)
	}
	linkPath := filepath.Join(t.TempDir(), ".claude.json")
	if err := os.Symlink(realPath, linkPath); err != nil {
		t.Skipf("cannot create a symlink here (Windows without developer mode?): %v", err)
	}

	result, err := claudeGrant(linkPath, claudeKeyPlan("D:/work/through-link"))
	if err != nil {
		t.Fatalf("claudeGrant: %v", err)
	}
	if !result.Written {
		t.Fatal("Written = false, want true")
	}

	info, err := os.Lstat(linkPath)
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode()&os.ModeSymlink == 0 {
		t.Fatalf("%s is no longer a symlink (mode %v)", linkPath, info.Mode())
	}
	if trusted, err := claudeTrusted(realPath, claudeKeyPlan("D:/work/through-link")); err != nil || !trusted {
		t.Errorf("link target: claudeTrusted = (%v, %v), want (true, nil)", trusted, err)
	}
	assertNoLockOrTempLeftover(t, linkPath)
	assertNoLockOrTempLeftover(t, realPath)
}

// C2 完了条件: 信頼済み（true）のキーがあると、ファイルの内容が 1 バイトも
// 変わらない。false の項目は Claude の既定値で回答ではないので、書き換える側
// （v0.9 リリース前の C10 C1 (c)。grant_test.go の
// TestGrantClaudeAcceptsAnEntryLeftAtTheDefaultFalse と下のテスト）。
func TestClaudeGrantDoesNotTouchFileWhenKeyIsTrusted(t *testing.T) {
	configPath := writeClaudeFixture(t, claudeFixtureWithTwoProjects)
	before := mustReadFile(t, configPath)

	result, err := claudeGrant(configPath, claudeKeyPlan("D:/work/SampleApp")) // hasTrustDialogAccepted: true
	if err != nil {
		t.Fatalf("claudeGrant: %v", err)
	}
	if result.Written || result.Existing != "trusted" {
		t.Fatalf("result = %+v, want Written=false and Existing=trusted", result)
	}
	if after := mustReadFile(t, configPath); !bytes.Equal(before, after) {
		t.Errorf("file changed even though the key was already trusted:\nbefore=%s\nafter=%s", before, after)
	}
	assertNoLockOrTempLeftover(t, configPath)
}

// false の項目だけが true になり、同じ項目のほかのキー・ほかの項目・ルートの
// 値は変わらない。
func TestClaudeGrantAcceptsOnlyTheDefaultFalseEntry(t *testing.T) {
	configPath := writeClaudeFixture(t, claudeFixtureWithTwoProjects)
	before := decodeWithNumbers(t, mustReadFile(t, configPath))

	result, err := claudeGrant(configPath, claudeKeyPlan("D:/work/sampleApp")) // {"hasTrustDialogAccepted": false, "extra": 1}
	if err != nil {
		t.Fatalf("claudeGrant: %v", err)
	}
	if !result.Written || result.Key != "D:/work/sampleApp" {
		t.Fatalf("result = %+v, want Written for D:/work/sampleApp", result)
	}
	after := decodeWithNumbers(t, mustReadFile(t, configPath))
	for _, key := range []string{"oauthAccount", "bigNumber", "nested"} {
		if !reflect.DeepEqual(before[key], after[key]) {
			t.Errorf("root key %q changed", key)
		}
	}
	projects := after["projects"].(map[string]any)
	want := map[string]any{"hasTrustDialogAccepted": true, "extra": json.Number("1")}
	if !reflect.DeepEqual(projects["D:/work/sampleApp"], want) {
		t.Errorf("entry = %#v, want %#v", projects["D:/work/sampleApp"], want)
	}
	if !reflect.DeepEqual(projects["D:/work/SampleApp"], before["projects"].(map[string]any)["D:/work/SampleApp"]) {
		t.Error("the other entry changed")
	}
	assertNoLockOrTempLeftover(t, configPath)
}

// Claude 自身の信頼の判定は、プロジェクトのキーのあとに作業フォルダから親へ
// 遡る。途中の親が信頼済みなら、Claude は確認を出さないので書かない。
func TestClaudeGrantDoesNotWriteWhenAParentIsTrusted(t *testing.T) {
	configPath := writeClaudeFixture(t, claudeFixtureWithTwoProjects)
	before := mustReadFile(t, configPath)
	plan := claudePlan{writeKey: "D:/work/SampleApp/sub", walk: []string{"D:/work/SampleApp/sub", "D:/work/SampleApp"}}

	result, err := claudeGrant(configPath, plan)
	if err != nil {
		t.Fatalf("claudeGrant: %v", err)
	}
	if result.Written || result.Existing != "trusted" || result.Key != "D:/work/SampleApp" {
		t.Errorf("result = %+v, want the trusted parent reported", result)
	}
	if !bytes.Equal(before, mustReadFile(t, configPath)) {
		t.Error("file changed")
	}
}

// claudeKeyPlan is a plan whose project key is key and that walks nothing
// else, for tests of the file handling alone.
func claudeKeyPlan(key string) claudePlan {
	return claudePlan{target: key, writeKey: key}
}

// C2 完了条件（陽性対照の裏返し）: claudeTrusted も同じ 2 状態を正しく読む。
func TestClaudeTrustedReadsBothStates(t *testing.T) {
	configPath := writeClaudeFixture(t, claudeFixtureWithTwoProjects)

	if trusted, err := claudeTrusted(configPath, claudeKeyPlan("D:/work/sampleApp")); err != nil || trusted {
		t.Errorf("claudeTrusted(sampleApp) = (%v, %v), want (false, nil)", trusted, err)
	}
	if trusted, err := claudeTrusted(configPath, claudeKeyPlan("D:/work/SampleApp")); err != nil || !trusted {
		t.Errorf("claudeTrusted(SampleApp) = (%v, %v), want (true, nil)", trusted, err)
	}
	if trusted, err := claudeTrusted(configPath, claudeKeyPlan("D:/work/never-seen")); err != nil || trusted {
		t.Errorf("claudeTrusted(never-seen) = (%v, %v), want (false, nil)", trusted, err)
	}
}

// C2 完了条件: 壊れた JSON のときは書かずにエラーで、ファイルは元のまま。
func TestClaudeGrantLeavesBrokenJSONUntouched(t *testing.T) {
	configPath := writeClaudeFixture(t, `{"projects": {"D:/x": `) // 意図的に壊す
	before, err := os.ReadFile(configPath)
	if err != nil {
		t.Fatal(err)
	}

	if _, err := claudeGrant(configPath, claudeKeyPlan("D:/x")); err == nil {
		t.Fatal("claudeGrant: want error for broken JSON, got nil")
	}

	after, err := os.ReadFile(configPath)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(before, after) {
		t.Error("broken JSON file was modified")
	}
	assertNoLockOrTempLeftover(t, configPath)
}

// C2 完了条件: 他者がロックを持っている間は待ち、5 秒で離されなければ
// エラー。タイムアウトはテストを遅くしないよう一時的に縮める。
func TestClaudeGrantWaitsForLockThenTimesOut(t *testing.T) {
	origTimeout, origInterval := claudeLockTimeout, claudeLockRetryInterval
	claudeLockTimeout = 200 * time.Millisecond
	claudeLockRetryInterval = 20 * time.Millisecond
	t.Cleanup(func() {
		claudeLockTimeout = origTimeout
		claudeLockRetryInterval = origInterval
	})

	configPath := writeClaudeFixture(t, claudeFixtureWithTwoProjects)
	lockDir := configPath + ".lock"
	if err := os.Mkdir(lockDir, 0o700); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.Remove(lockDir) }) // このロックは他者役なので自分で片付ける

	start := time.Now()
	if _, err := claudeGrant(configPath, claudeKeyPlan("D:/x")); err == nil {
		t.Fatal("claudeGrant: want error when lock is held, got nil")
	}
	if elapsed := time.Since(start); elapsed < claudeLockTimeout {
		t.Errorf("returned after %s, want at least %s (should have waited)", elapsed, claudeLockTimeout)
	}
}

// C2 完了条件: ロックの更新時刻が 11 秒前なら取り直して書ける。
func TestClaudeGrantReclaimsAStaleLock(t *testing.T) {
	configPath := writeClaudeFixture(t, claudeFixtureWithTwoProjects)
	lockDir := configPath + ".lock"
	if err := os.Mkdir(lockDir, 0o700); err != nil {
		t.Fatal(err)
	}
	stale := time.Now().Add(-11 * time.Second)
	if err := os.Chtimes(lockDir, stale, stale); err != nil {
		t.Fatal(err)
	}

	result, err := claudeGrant(configPath, claudeKeyPlan("D:/tmp/sample-repo/cases/reclaimed"))
	if err != nil {
		t.Fatalf("claudeGrant: %v", err)
	}
	if !result.Written {
		t.Error("Written = false, want true (stale lock should have been reclaimed)")
	}
	assertNoLockOrTempLeftover(t, configPath)
}
