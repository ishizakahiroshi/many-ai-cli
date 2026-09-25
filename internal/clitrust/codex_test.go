package clitrust

import (
	"bytes"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"testing"
)

// synthetic fixture only — comments・空行・MCP の表を含む合成データ。
const codexFixtureWithComments = `# codex config.toml (synthetic fixture)
model = "gpt-5-codex"

[mcp_servers.example]
command = "echo"
args = ["hello"]

[projects]
[projects.'D:/work/existing-trusted']
trust_level = "trusted"

[projects.'D:/work/existing-untrusted']
trust_level = "untrusted"
`

func writeCodexFixture(t *testing.T, body string) string {
	t.Helper()
	dir := t.TempDir()
	configPath := filepath.Join(dir, "config.toml")
	if body != "" {
		if err := os.WriteFile(configPath, []byte(body), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	return configPath
}

// C3 完了条件: 既存の内容（コメント・空行・MCP の表を含む合成の fixture）が、
// 追記の前の部分について 1 バイトも変わらない。
func TestCodexGrantPreservesExistingContentAsPrefix(t *testing.T) {
	configPath := writeCodexFixture(t, codexFixtureWithComments)

	result, err := codexGrant(configPath, codexKeyPlan("d:/tmp/sample-repo/cases/new"))
	if err != nil {
		t.Fatalf("codexGrant: %v", err)
	}
	if !result.Written {
		t.Fatalf("Written = false, want true")
	}

	updated, err := os.ReadFile(configPath)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.HasPrefix(updated, []byte(codexFixtureWithComments)) {
		t.Errorf("existing content was not preserved as a byte-identical prefix:\noriginal=%s\nupdated=%s", codexFixtureWithComments, updated)
	}

	// 追記後、既存の 2 件も引き続き読める。
	doc, _, err := readCodexDoc(configPath)
	if err != nil {
		t.Fatalf("readCodexDoc after append: %v", err)
	}
	if doc.Projects["D:/work/existing-trusted"].TrustLevel != "trusted" {
		t.Error("existing trusted entry lost after append")
	}
	if doc.Projects["D:/work/existing-untrusted"].TrustLevel != "untrusted" {
		t.Error("existing untrusted entry lost after append")
	}
	if doc.Projects["d:/tmp/sample-repo/cases/new"].TrustLevel != "trusted" {
		t.Error("new entry not trusted after append")
	}
}

// C3 完了条件: キーが既にある（trusted / untrusted）と、ファイルが変わらない。
func TestCodexGrantDoesNotTouchFileWhenKeyExists(t *testing.T) {
	for _, tc := range []struct {
		key      string
		existing string
	}{
		{"D:/work/existing-trusted", "trusted"},
		{"D:/work/existing-untrusted", "untrusted"},
	} {
		t.Run(tc.existing, func(t *testing.T) {
			configPath := writeCodexFixture(t, codexFixtureWithComments)
			before, err := os.ReadFile(configPath)
			if err != nil {
				t.Fatal(err)
			}

			result, err := codexGrant(configPath, codexKeyPlan(tc.key))
			if err != nil {
				t.Fatalf("codexGrant: %v", err)
			}
			if result.Written {
				t.Fatal("Written = true, want false (key already present)")
			}
			if result.Existing != tc.existing {
				t.Errorf("Existing = %q, want %q", result.Existing, tc.existing)
			}

			after, err := os.ReadFile(configPath)
			if err != nil {
				t.Fatal(err)
			}
			if !bytes.Equal(before, after) {
				t.Errorf("file changed even though the key already existed:\nbefore=%s\nafter=%s", before, after)
			}
		})
	}
}

// C3 完了条件: ' を含むパスでも、追記後に TOML として読めてキーが引ける。
func TestCodexGrantEscapesASingleQuoteInTheKey(t *testing.T) {
	configPath := writeCodexFixture(t, codexFixtureWithComments)
	key := `d:\o'brien\project`

	result, err := codexGrant(configPath, codexKeyPlan(key))
	if err != nil {
		t.Fatalf("codexGrant: %v", err)
	}
	if !result.Written {
		t.Fatal("Written = false, want true")
	}

	doc, _, err := readCodexDoc(configPath)
	if err != nil {
		t.Fatalf("readCodexDoc after append: %v", err)
	}
	if doc.Projects[key].TrustLevel != "trusted" {
		t.Errorf("key with a single quote did not round-trip: got entry %+v", doc.Projects[key])
	}
}

// C3 完了条件: 壊れた TOML のときは書かずにエラー。
func TestCodexGrantLeavesBrokenTOMLUntouched(t *testing.T) {
	configPath := writeCodexFixture(t, "this = [is, not, valid toml")
	before, err := os.ReadFile(configPath)
	if err != nil {
		t.Fatal(err)
	}

	if _, err := codexGrant(configPath, codexKeyPlan("d:/x")); err == nil {
		t.Fatal("codexGrant: want error for broken TOML, got nil")
	}

	after, err := os.ReadFile(configPath)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(before, after) {
		t.Error("broken TOML file was modified")
	}
}

// 追記すると TOML として読めなくなる形（projects をインライン表で書いた
// ファイルには [projects.'…'] を足せない）では、追記した分だけを切り詰めて
// 元に戻し、エラーを返す。ファイルは 1 バイトも変わらない。
func TestCodexGrantRollsBackAnAppendThatBreaksTheFile(t *testing.T) {
	original := "model = \"gpt-5-codex\"\nprojects = { 'd:\\dev\\a' = { trust_level = \"trusted\" } }\n"
	configPath := writeCodexFixture(t, original)

	if _, err := codexGrant(configPath, codexKeyPlan(`d:\work\b`)); err == nil {
		t.Fatal("codexGrant: want error when the appended table cannot be read back, got nil")
	}
	if got := mustReadFile(t, configPath); !bytes.Equal(got, []byte(original)) {
		t.Errorf("file was not restored:\nwant=%q\ngot =%q", original, got)
	}
}

// 書いたときの Result は Written=true で、Existing は空（Result の定義どおり）。
func TestCodexGrantReportsAWriteWithoutExisting(t *testing.T) {
	configPath := writeCodexFixture(t, codexFixtureWithComments)
	result, err := codexGrant(configPath, codexKeyPlan(`d:\work\fresh`))
	if err != nil {
		t.Fatalf("codexGrant: %v", err)
	}
	if !result.Written || result.Existing != "" {
		t.Errorf("result = %+v, want Written=true and Existing empty", result)
	}
}

// config.toml が存在しないとき（codex は無くても起動する）は、空として扱い
// 新規作成できる。
func TestCodexGrantCreatesTheFileWhenMissing(t *testing.T) {
	dir := t.TempDir()
	configPath := filepath.Join(dir, "config.toml")

	result, err := codexGrant(configPath, codexKeyPlan("d:/x"))
	if err != nil {
		t.Fatalf("codexGrant: %v", err)
	}
	if !result.Written {
		t.Fatal("Written = false, want true")
	}
	if !bytes.HasPrefix(bytes.TrimLeft(mustReadFile(t, configPath), "\n"), []byte("[projects.")) {
		t.Errorf("new file does not start with the trust table: %s", mustReadFile(t, configPath))
	}
}

func TestCodexTrustedReadsBothStates(t *testing.T) {
	configPath := writeCodexFixture(t, codexFixtureWithComments)

	if trusted, err := codexTrusted(configPath, codexKeyPlan("D:/work/existing-trusted")); err != nil || !trusted {
		t.Errorf("codexTrusted(existing-trusted) = (%v, %v), want (true, nil)", trusted, err)
	}
	if trusted, err := codexTrusted(configPath, codexKeyPlan("D:/work/existing-untrusted")); err != nil || trusted {
		t.Errorf("codexTrusted(existing-untrusted) = (%v, %v), want (false, nil)", trusted, err)
	}
	if trusted, err := codexTrusted(configPath, codexKeyPlan("D:/work/never-seen")); err != nil || trusted {
		t.Errorf("codexTrusted(never-seen) = (%v, %v), want (false, nil)", trusted, err)
	}
}

// codex 自身は config.toml が無くても起動するので、Trusted も「無ければ
// 未信頼」として扱い、エラーにしない。
func TestCodexTrustedTreatsAMissingFileAsUntrusted(t *testing.T) {
	dir := t.TempDir()
	configPath := filepath.Join(dir, "config.toml")
	if trusted, err := codexTrusted(configPath, codexKeyPlan("d:/x")); err != nil || trusted {
		t.Errorf("codexTrusted(missing file) = (%v, %v), want (false, nil)", trusted, err)
	}
}

// 同じキーの Grant が同時に 2 本走っても config.toml を壊さない。以前は両方が追記して
// 表が二重になり、両方が巻き戻しに入って、後から切った側が自分の追記前の長さまで
// ファイルを伸ばし、末尾が NUL で埋まって codex が読めなくなった（v0.9 リリース前の
// 敵対レビュー A の F1。200 回中 16 回再現）。
func TestCodexGrantConcurrentSameKeyKeepsTheFileReadable(t *testing.T) {
	for round := 0; round < 50; round++ {
		configPath := writeCodexFixture(t, codexFixtureWithComments)
		var wg sync.WaitGroup
		start := make(chan struct{})
		for i := 0; i < 2; i++ {
			wg.Add(1)
			go func() {
				defer wg.Done()
				<-start
				_, _ = codexGrant(configPath, codexKeyPlan(`d:\work\same`))
			}()
		}
		close(start)
		wg.Wait()
		data := mustReadFile(t, configPath)
		if bytes.IndexByte(data, 0) >= 0 {
			t.Fatalf("round %d: NUL in config.toml", round)
		}
		doc, _, err := readCodexDoc(configPath)
		if err != nil {
			t.Fatalf("round %d: %v", round, err)
		}
		if doc.Projects[`d:\work\same`].TrustLevel != "trusted" {
			t.Fatalf("round %d: the key was not left trusted: %+v", round, doc.Projects)
		}
	}
}

// 巻き戻しは、ファイルがちょうど「追記前 + 自分の追記」のときだけ切る。追記の後に
// 別の書き手（codex 本体）が書き換えていたら触らない。切ると他者の書いた内容を
// 落とすか、短くなったファイルを NUL で伸ばす。
func TestCodexRollbackOnlyRemovesItsOwnAppend(t *testing.T) {
	const original = "model = \"gpt-5-codex\"\n"
	block := codexTrustBlock(`d:\work\b`, false)

	t.Run("ours is the tail", func(t *testing.T) {
		configPath := writeCodexFixture(t, original+block)
		rollbackCodexAppend(configPath, false, int64(len(original)), block)
		if got := string(mustReadFile(t, configPath)); got != original {
			t.Errorf("got %q, want %q", got, original)
		}
	})
	t.Run("someone appended after us", func(t *testing.T) {
		body := original + block + "[other]\nx = 1\n"
		configPath := writeCodexFixture(t, body)
		rollbackCodexAppend(configPath, false, int64(len(original)), block)
		if got := string(mustReadFile(t, configPath)); got != body {
			t.Errorf("file was cut: got %q", got)
		}
	})
	t.Run("someone rewrote the file shorter", func(t *testing.T) {
		const rewritten = "model = \"x\"\n"
		configPath := writeCodexFixture(t, rewritten)
		rollbackCodexAppend(configPath, false, int64(len(original)), block)
		if got := string(mustReadFile(t, configPath)); got != rewritten {
			t.Errorf("file was changed: got %q", got)
		}
	})
	t.Run("we created the file", func(t *testing.T) {
		first := codexTrustBlock(`d:\work\b`, true)
		configPath := writeCodexFixture(t, first)
		rollbackCodexAppend(configPath, true, 0, first)
		if _, err := os.Stat(configPath); !os.IsNotExist(err) {
			t.Errorf("the file this grant created was not removed: %v", err)
		}
	})
	t.Run("someone else wrote into the file we created", func(t *testing.T) {
		first := codexTrustBlock(`d:\work\b`, true)
		body := first + "[other]\nx = 1\n"
		configPath := writeCodexFixture(t, body)
		rollbackCodexAppend(configPath, true, 0, first)
		if got := string(mustReadFile(t, configPath)); got != body {
			t.Errorf("file was removed or cut: got %q", got)
		}
	})
}

// codexKeyPlan is a plan whose only lookup key and write key is key, with no
// untrusted guard, for tests of the file handling alone.
func codexKeyPlan(key string) codexPlan {
	return codexPlan{target: key, writeKey: key, lookup: []string{key}}
}

// codex の検索は、完全一致の次に「小文字にすると一致する」キーを見る（Windows）。
// 大文字小文字だけ違う信頼済みの項目があれば、新しい表を足さない。
func TestCodexGrantFindsAnEntryInAnotherCaseOnWindows(t *testing.T) {
	if runtime.GOOS != "windows" {
		t.Skip("codex folds letter case on Windows only")
	}
	configPath := writeCodexFixture(t, "[projects.'D:\\Work\\Repo']\ntrust_level = \"trusted\"\n")
	before := mustReadFile(t, configPath)

	result, err := codexGrant(configPath, codexKeyPlan(`d:\work\repo`))
	if err != nil {
		t.Fatalf("codexGrant: %v", err)
	}
	if result.Written || result.Existing != "trusted" || result.Key != `D:\Work\Repo` {
		t.Errorf("result = %+v, want the existing D:\\Work\\Repo reported as trusted", result)
	}
	if !bytes.Equal(before, mustReadFile(t, configPath)) {
		t.Error("file changed")
	}
}

// 未信頼の項目が作業フォルダかその上にあれば書かない。横や下にある項目は
// 関係ない。ドライブの根・末尾の区切り・"/" 区切りで書かれた項目も同じ扱い。
func TestCodexUntrustedAtOrAbove(t *testing.T) {
	sep := string(filepath.Separator)
	root := filepath.VolumeName(os.TempDir()) + sep
	folder := filepath.Join(root, "work", "repo", "sub")
	guard := []string{codexComparable(folder)}
	for name, tc := range map[string]struct {
		key     string
		level   string
		blocked bool
	}{
		"same folder":      {folder, "untrusted", true},
		"parent":           {filepath.Join(root, "work", "repo"), "untrusted", true},
		"parent, trailing": {filepath.Join(root, "work", "repo") + sep, "untrusted", true},
		"filesystem root":  {root, "untrusted", true},
		"trusted parent":   {filepath.Join(root, "work", "repo"), "trusted", false},
		"sibling":          {filepath.Join(root, "work", "repo", "other"), "untrusted", false},
		"prefix only":      {filepath.Join(root, "work", "rep"), "untrusted", false},
		"below":            {filepath.Join(folder, "deeper"), "untrusted", false},
	} {
		t.Run(name, func(t *testing.T) {
			projects := map[string]codexProjectEntry{tc.key: {TrustLevel: tc.level}}
			if _, blocked := codexUntrustedAtOrAbove(projects, guard); blocked != tc.blocked {
				t.Errorf("blocked = %v, want %v", blocked, tc.blocked)
			}
		})
	}
	if runtime.GOOS == "windows" {
		projects := map[string]codexProjectEntry{strings.ToUpper(strings.ReplaceAll(filepath.Join(root, "work", "repo"), `\`, "/")): {TrustLevel: "untrusted"}}
		if _, blocked := codexUntrustedAtOrAbove(projects, guard); !blocked {
			t.Error("an upper-case, slash-separated untrusted parent was not recognized")
		}
	}
}

// codex の小文字化は ASCII だけ（Rust の to_ascii_lowercase）。
func TestCodexKeyLowercasesASCIIOnly(t *testing.T) {
	if got, want := asciiLower(`D:\Ärger\ÖL\Abc`), `d:\Ärger\Öl\abc`; got != want {
		t.Errorf("asciiLower = %q, want %q", got, want)
	}
}

func mustReadFile(t *testing.T, path string) []byte {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return data
}
