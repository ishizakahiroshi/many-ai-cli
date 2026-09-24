package clitrust

import (
	"bytes"
	"os"
	"path/filepath"
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

	result, err := codexGrant(configPath, "d:/tmp/sample-repo/cases/new")
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

			result, err := codexGrant(configPath, tc.key)
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

	result, err := codexGrant(configPath, key)
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

	if _, err := codexGrant(configPath, "d:/x"); err == nil {
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

	if _, err := codexGrant(configPath, `d:\work\b`); err == nil {
		t.Fatal("codexGrant: want error when the appended table cannot be read back, got nil")
	}
	if got := mustReadFile(t, configPath); !bytes.Equal(got, []byte(original)) {
		t.Errorf("file was not restored:\nwant=%q\ngot =%q", original, got)
	}
}

// 書いたときの Result は Written=true で、Existing は空（Result の定義どおり）。
func TestCodexGrantReportsAWriteWithoutExisting(t *testing.T) {
	configPath := writeCodexFixture(t, codexFixtureWithComments)
	result, err := codexGrant(configPath, `d:\work\fresh`)
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

	result, err := codexGrant(configPath, "d:/x")
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

	if trusted, err := codexTrusted(configPath, "D:/work/existing-trusted"); err != nil || !trusted {
		t.Errorf("codexTrusted(existing-trusted) = (%v, %v), want (true, nil)", trusted, err)
	}
	if trusted, err := codexTrusted(configPath, "D:/work/existing-untrusted"); err != nil || trusted {
		t.Errorf("codexTrusted(existing-untrusted) = (%v, %v), want (false, nil)", trusted, err)
	}
	if trusted, err := codexTrusted(configPath, "D:/work/never-seen"); err != nil || trusted {
		t.Errorf("codexTrusted(never-seen) = (%v, %v), want (false, nil)", trusted, err)
	}
}

// codex 自身は config.toml が無くても起動するので、Trusted も「無ければ
// 未信頼」として扱い、エラーにしない。
func TestCodexTrustedTreatsAMissingFileAsUntrusted(t *testing.T) {
	dir := t.TempDir()
	configPath := filepath.Join(dir, "config.toml")
	if trusted, err := codexTrusted(configPath, "d:/x"); err != nil || trusted {
		t.Errorf("codexTrusted(missing file) = (%v, %v), want (false, nil)", trusted, err)
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
