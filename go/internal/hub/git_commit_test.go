package hub

import (
	"os"
	"path/filepath"
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

// TestSuggestCommitMessagePrefix は変更種別から conventional commit の <type>(<scope>):
// が正しく出し分けられること（方針1）を検証する。scope は最深共通ディレクトリの
// 非汎用セグメント（internal / src / cmd / pkg / lib を除いた最深）から取る。
func TestSuggestCommitMessagePrefix(t *testing.T) {
	cases := []struct {
		name       string
		files      []gitStatusFile
		diff       string
		wantPrefix string
	}{
		{
			name:       "新規ファイルのみは feat(scope)",
			files:      []gitStatusFile{sf("A", "internal/hub/new_feature.go")},
			wantPrefix: "feat(hub):",
		},
		{
			name:       "新規関数追加は feat(scope)",
			files:      []gitStatusFile{sf("M", "internal/hub/server.go")},
			diff:       "+++ b/internal/hub/server.go\n+func DoNewThing() {\n",
			wantPrefix: "feat(hub):",
		},
		{
			name:       "既存コードの編集だけは refactor(scope)（feat にしない）",
			files:      []gitStatusFile{sf("M", "internal/hub/server.go")},
			diff:       "+++ b/internal/hub/server.go\n+\tx := 1\n-\tx := 2\n",
			wantPrefix: "refactor(hub):",
		},
		{
			name:       "依存ファイルのみは chore(deps)（追加 scope は付けない）",
			files:      []gitStatusFile{sf("M", "go.mod"), sf("M", "go.sum")},
			wantPrefix: "chore(deps):",
		},
		{
			name:       "ドキュメントのみは docs（scope=docs は prefix と重複するので省く）",
			files:      []gitStatusFile{sf("M", "docs/guide.md")},
			wantPrefix: "docs:",
		},
		{
			name:       "テストのみは test(scope)",
			files:      []gitStatusFile{sf("M", "internal/hub/server_test.go")},
			wantPrefix: "test(hub):",
		},
		{
			name:       "CSS のみは style",
			files:      []gitStatusFile{sf("M", "web/src/styles/chat.css")},
			wantPrefix: "style",
		},
		{
			name:       "コード削除のみは refactor(scope)",
			files:      []gitStatusFile{sf("D", "internal/hub/old.go")},
			wantPrefix: "refactor(hub):",
		},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			subject, _ := suggestCommitMessage(c.files, "", c.diff, "", "ja")
			if !strings.HasPrefix(subject, c.wantPrefix) {
				t.Errorf("prefix mismatch: got %q, want prefix %q", subject, c.wantPrefix)
			}
		})
	}
}

// TestSuggestCommitMessageDefaultSubject は無情報な「scope を更新」を避け、
// 代表的な変更ファイル名 + 選定された動詞で subject が組まれること（ハイブリッド型 /
// verb=refactor / scope=web ※src は汎用として skip）を検証する。
func TestSuggestCommitMessageDefaultSubject(t *testing.T) {
	files := []gitStatusFile{
		sf("M", "web/src/app.ts"),
		sf("M", "web/src/app/state.ts"),
		sf("M", "web/src/styles.css"),
	}
	subjectJa, _ := suggestCommitMessage(files, "", "", "", "ja")
	if want := "refactor(web): app.ts ほか 2 件 を整理"; subjectJa != want {
		t.Errorf("ja default subject: got %q, want %q", subjectJa, want)
	}

	subjectEn, _ := suggestCommitMessage(files, "", "", "", "en")
	if want := "refactor(web): rework app.ts (+2 more)"; subjectEn != want {
		t.Errorf("en default subject: got %q, want %q", subjectEn, want)
	}
}

// TestSuggestCommitMessageVerbs は 9 種＋update の動詞が diff から正しく選ばれる
// こと（add / remove / rename / move / bump / simplify / handle / refactor / update）
// を代表ケースで検証する。
func TestSuggestCommitMessageVerbs(t *testing.T) {
	cases := []struct {
		name   string
		files  []gitStatusFile
		diff   string
		wantJa string
		wantEn string
	}{
		{
			name:   "add: 新規ファイル 1 件",
			files:  []gitStatusFile{sf("A", "internal/hub/new_feature.go")},
			wantJa: "feat(hub): new_feature.go を追加",
			wantEn: "feat(hub): add new_feature.go",
		},
		{
			name:   "add: 新規関数",
			files:  []gitStatusFile{sf("M", "internal/hub/server.go")},
			diff:   "+++ b/internal/hub/server.go\n+func DoNewThing() {\n",
			wantJa: "feat(hub): DoNewThing を追加",
			wantEn: "feat(hub): add DoNewThing",
		},
		{
			name:   "remove: 削除のみ",
			files:  []gitStatusFile{sf("D", "internal/hub/old.go")},
			wantJa: "refactor(hub): old.go を削除",
			wantEn: "refactor(hub): remove old.go",
		},
		{
			name:  "rename: リネームのみ",
			files: []gitStatusFile{sf("R", "internal/hub/newname.go")},
			diff:  "rename from internal/hub/oldname.go\nrename to internal/hub/newname.go\n",
			// prefix は rename-only 経由で refactor、scope=hub。
			wantJa: "refactor(hub): oldname.go → newname.go に改名",
			wantEn: "refactor(hub): rename oldname.go → newname.go",
		},
		{
			name:  "move: 同名関数が別ファイルで加減",
			files: []gitStatusFile{sf("M", "internal/hub/a.go"), sf("M", "internal/hub/b.go")},
			diff: "+++ b/internal/hub/a.go\n+func MovedFn() {\n" +
				"+++ b/internal/hub/b.go\n-func MovedFn() {\n",
			wantJa: "feat(hub): MovedFn を移動",
			wantEn: "feat(hub): move MovedFn",
		},
		{
			// 同一ファイル内で関数を書き換える（既存の関数を丸ごと差し替え）ケースは
			// move に昇格させず add に落とす（addedFuncSites==deletedFuncSites なので）。
			name:  "rewrite: 同一ファイル内の関数書き換えは move にしない",
			files: []gitStatusFile{sf("M", "internal/hub/a.go")},
			diff: "+++ b/internal/hub/a.go\n" +
				"-func RewrittenFn() {\n" +
				"+func RewrittenFn() {\n",
			wantJa: "feat(hub): RewrittenFn を追加",
			wantEn: "feat(hub): add RewrittenFn",
		},
		{
			name:   "bump: deps のみ",
			files:  []gitStatusFile{sf("M", "go.mod"), sf("M", "go.sum")},
			wantJa: "chore(deps): go.mod ほか 1 件 を更新",
			wantEn: "chore(deps): bump go.mod (+1 more)",
		},
		{
			name:  "simplify: LOC が明確に減少",
			files: []gitStatusFile{sf("M", "internal/hub/server.go")},
			// locDeleted >= 15 かつ locAdded*3 < locDeleted*2 の閾値を満たす（削除 20 / 追加 5）。
			diff: "+++ b/internal/hub/server.go\n" +
				strings.Repeat("-old line\n", 20) +
				strings.Repeat("+new line\n", 5),
			wantJa: "refactor(hub): server.go を簡潔化",
			wantEn: "refactor(hub): simplify server.go",
		},
		{
			name:  "handle: err/throw/catch 追加が支配的",
			files: []gitStatusFile{sf("M", "internal/hub/server.go")},
			diff: "+++ b/internal/hub/server.go\n" +
				"+\tif err != nil {\n" +
				"+\t\tif err != nil {\n" +
				"+\t\t\tif err != nil {\n",
			wantJa: "refactor(hub): server.go にエラー処理を追加",
			wantEn: "refactor(hub): handle errors in server.go",
		},
		{
			name:   "update: docs の M-only",
			files:  []gitStatusFile{sf("M", "docs/guide.md")},
			wantJa: "docs: guide.md を更新",
			wantEn: "docs: update guide.md",
		},
		{
			name:   "update: test の M-only は test(scope) + update",
			files:  []gitStatusFile{sf("M", "internal/hub/server_test.go")},
			wantJa: "test(hub): server_test.go を更新",
			wantEn: "test(hub): update server_test.go",
		},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			gotJa, _ := suggestCommitMessage(c.files, "", c.diff, "", "ja")
			if gotJa != c.wantJa {
				t.Errorf("ja: got %q, want %q", gotJa, c.wantJa)
			}
			gotEn, _ := suggestCommitMessage(c.files, "", c.diff, "", "en")
			if gotEn != c.wantEn {
				t.Errorf("en: got %q, want %q", gotEn, c.wantEn)
			}
		})
	}
}

func TestSuggestCommitMessageUsesDominantChange(t *testing.T) {
	files := []gitStatusFile{sf("A", "web/src/data/admission.ts")}
	weights := map[string]int{"web/src/data/admission.ts": 3}
	for i := 0; i < 10; i++ {
		path := "web/src/components/Changed" + string(rune('A'+i)) + ".tsx"
		if i == 0 {
			path = "web/src/components/SchoolDetailSheet.tsx"
			weights[path] = 48
		} else {
			weights[path] = i + 4
		}
		files = append(files, sf("M", path))
	}
	diff := "+++ b/web/src/data/admission.ts\n" +
		"+export interface AdmissionInfo { score: number }\n" +
		"+++ b/web/src/components/SchoolDetailSheet.tsx\n" +
		"+const title = formatSchoolTitle(school)\n"

	gotJa, _ := suggestCommitMessageWithWeights(files, "", diff, "", "ja", weights)
	wantJa := "feat(web): SchoolDetailSheet.tsx ほか 9 件 を変更（admission.ts 新規）"
	if gotJa != wantJa {
		t.Fatalf("ja: got %q, want %q", gotJa, wantJa)
	}
	gotEn, _ := suggestCommitMessageWithWeights(files, "", diff, "", "en", weights)
	wantEn := "feat(web): change SchoolDetailSheet.tsx (+9 more) (new: admission.ts)"
	if gotEn != wantEn {
		t.Fatalf("en: got %q, want %q", gotEn, wantEn)
	}
}

func TestSuggestCommitMessageUsesSymbolFromDominantModifiedFile(t *testing.T) {
	files := []gitStatusFile{
		sf("A", "web/src/data/admission.ts"),
		sf("M", "web/src/components/SchoolDetailSheet.tsx"),
	}
	weights := map[string]int{
		"web/src/data/admission.ts":                3,
		"web/src/components/SchoolDetailSheet.tsx": 48,
	}
	diff := "+++ b/web/src/data/admission.ts\n" +
		"+export interface AdmissionInfo { score: number }\n" +
		"+++ b/web/src/components/SchoolDetailSheet.tsx\n" +
		"+export function renderSchoolDetail() {}\n"

	for _, language := range []string{"ja", "en"} {
		subject, _ := suggestCommitMessageWithWeights(files, "", diff, "", language, weights)
		if !strings.Contains(subject, "renderSchoolDetail") {
			t.Fatalf("%s subject did not use the dominant file's symbol: %q", language, subject)
		}
		if strings.Contains(subject, "AdmissionInfo") {
			t.Fatalf("%s subject used a symbol from the smaller new file: %q", language, subject)
		}
	}
}

func TestSuggestCommitMessageKeepsDominantAddedFile(t *testing.T) {
	files := []gitStatusFile{
		sf("A", "web/src/new-feature.ts"),
		sf("M", "web/src/existing.ts"),
	}
	weights := map[string]int{
		"web/src/new-feature.ts": 80,
		"web/src/existing.ts":    5,
	}
	for _, language := range []string{"ja", "en"} {
		subject, _ := suggestCommitMessageWithWeights(files, "", "", "", language, weights)
		if !strings.Contains(subject, "new-feature.ts") {
			t.Fatalf("%s subject did not use dominant added file: %q", language, subject)
		}
	}
}

func TestAnalyzeCommitChangesExtractsMultipleLanguages(t *testing.T) {
	files := []gitStatusFile{
		sf("M", "web/src/feature.ts"),
		sf("M", "tools/report.py"),
	}
	diff := "+++ b/web/src/feature.ts\n" +
		"+export async function loadFeature() {\n" +
		"+export const featureName = 'x'\n" +
		"+export default class FeatureController {}\n" +
		"+export interface FeatureOptions {}\n" +
		"+++ b/tools/report.py\n" +
		"+def render_report():\n" +
		"+class ReportBuilder:\n"
	a := analyzeCommitChanges(files, diff)

	for _, want := range []string{"loadFeature", "featureName", "FeatureController", "render_report"} {
		if !containsString(a.funcs, want) {
			t.Errorf("funcs %v do not contain %q", a.funcs, want)
		}
	}
	for _, want := range []string{"FeatureOptions", "ReportBuilder"} {
		if !containsString(a.types, want) {
			t.Errorf("types %v do not contain %q", a.types, want)
		}
	}
}

func TestAnalyzeCommitChangesExtractsRoutesAndI18n(t *testing.T) {
	files := []gitStatusFile{
		sf("M", "web/src/server.ts"),
		sf("M", "api/routes.py"),
		sf("M", "web/src/i18n/en.ts"),
	}
	diff := "+++ b/web/src/server.ts\n" +
		"+router.get('/health', handler)\n" +
		"-router.post('/moved', oldHandler)\n" +
		"+router.post('/moved', newHandler)\n" +
		"+++ b/api/routes.py\n" +
		"+@app.post('/items')\n" +
		"+++ b/web/src/i18n/en.ts\n" +
		"+  'setup.ready': 'Ready',\n" +
		"+  setup_failed: 'Failed',\n"
	a := analyzeCommitChanges(files, diff)

	if !containsString(a.routes, "/health") || !containsString(a.routes, "/items") {
		t.Fatalf("routes = %v, want /health and /items", a.routes)
	}
	if containsString(a.routes, "/moved") || containsString(a.removedRts, "/moved") {
		t.Fatalf("moved route was not cancelled: added=%v removed=%v", a.routes, a.removedRts)
	}
	if a.i18nKeys != 2 {
		t.Fatalf("i18nKeys = %d, want 2", a.i18nKeys)
	}
	for _, language := range []string{"ja", "en"} {
		subject, _ := suggestCommitMessage(files, "", diff, "", language)
		if !strings.Contains(subject, "/health") {
			t.Fatalf("%s subject did not use extracted route: %q", language, subject)
		}
	}
}

func TestCountUntrackedFileLines(t *testing.T) {
	root := t.TempDir()
	path := filepath.Join(root, "new.ts")
	if err := os.WriteFile(path, []byte("one\ntwo\nthree"), 0o600); err != nil {
		t.Fatal(err)
	}
	if got := countUntrackedFileLines(root, "new.ts"); got != 3 {
		t.Fatalf("line count = %d, want 3", got)
	}
	if err := os.WriteFile(filepath.Join(root, "binary.dat"), []byte{'a', 0, 'b'}, 0o600); err != nil {
		t.Fatal(err)
	}
	if got := countUntrackedFileLines(root, "binary.dat"); got != 0 {
		t.Fatalf("binary line count = %d, want 0", got)
	}
}

func containsString(items []string, want string) bool {
	for _, item := range items {
		if item == want {
			return true
		}
	}
	return false
}
