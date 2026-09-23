package hub

import (
	"go/ast"
	"go/parser"
	"go/token"
	"io/fs"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/proto"
)

// 承認の表示は Hub が持つ保留中の記録を描くだけ（approval_record.go の冒頭）。画面は端末の
// 文字から承認を作らず、「保留中」も記録からだけ出す（docs/local/plan_approval-display-single-source.md）。
// このファイルは、その形を Hub 側で固定する。画面側の固定は scripts/check-approval-display-source.mjs。

// legacyApprovalMessageTypes は、記録へ切り替える前に承認ごとに送っていたメッセージと、画面が
// 「承認が見えている」と申告していたメッセージの種類。どれも 2026-09-23 に撤去した。
// 送り直すと、画面が記録と別の経路で承認を描く（または「保留中」を決める）余地が戻る。
var legacyApprovalMessageTypes = []string{
	"approval_detected",
	"approval_marker",
	"approval_cleared",
	"session_hint",
}

// goSourceFiles は internal/ と cmd/ の、テストでない .go ファイルを返す。
func goSourceFiles(t *testing.T) []string {
	t.Helper()
	var files []string
	for _, root := range []string{filepath.Join("..", "..", "internal"), filepath.Join("..", "..", "cmd")} {
		err := filepath.WalkDir(root, func(path string, d fs.DirEntry, err error) error {
			if err != nil {
				return err
			}
			if d.IsDir() || !strings.HasSuffix(path, ".go") || strings.HasSuffix(path, "_test.go") {
				return nil
			}
			files = append(files, path)
			return nil
		})
		if err != nil {
			t.Fatal(err)
		}
	}
	if len(files) == 0 {
		t.Fatal("走査対象の .go が 0 件。走査の起点が実装とずれている")
	}
	return files
}

// 旧メッセージの種類名を、Go のソースに文字列として書かない。送る側（broadcast）も受ける側
// （UI からのメッセージの switch）も、種類名の文字列が無ければ作れない。
func TestApprovalLegacyMessagesAreNotSent(t *testing.T) {
	legacy := map[string]bool{}
	for _, typ := range legacyApprovalMessageTypes {
		legacy[typ] = true
	}
	for _, file := range goSourceFiles(t) {
		fset := token.NewFileSet()
		parsed, err := parser.ParseFile(fset, file, nil, 0)
		if err != nil {
			t.Fatal(err)
		}
		ast.Inspect(parsed, func(n ast.Node) bool {
			lit, ok := n.(*ast.BasicLit)
			if !ok || lit.Kind != token.STRING {
				return true
			}
			if value, err := strconv.Unquote(lit.Value); err == nil && legacy[value] {
				t.Errorf("%s: 撤去したメッセージの種類 %q が戻っている（承認は approval_state だけで知らせる）", fset.Position(lit.Pos()), value)
			}
			return true
		})
	}

	// 実際に開いて閉じる経路を一通り通し、承認の知らせが approval_state だけであることを確かめる。
	s := newTestServer()
	sent := captureUIBroadcasts(s)

	registerTestSession(s, 1, "codex")
	native := syntheticNativeApproval("Run git status?")
	s.handleNativeApprovalDetection(1, native)
	record := s.sessions[1].pendingApproval
	s.markNativeApprovalConsumed(proto.Message{SessionID: 1, ApprovalSig: record.Sig, ApprovalCandidateKey: record.CandidateKey, ApprovalSourceEpoch: record.SourceEpoch})
	s.handleNativeApprovalDetection(1, syntheticNativeApproval("Run git diff?"))
	for i := 0; i < nativeApprovalClearMissLimit; i++ {
		s.handleNativeApprovalDetection(1, nil)
	}

	registerTestSession(s, 2, "grok")
	s.maybeBroadcastApprovalMarker(2, syntheticMarker(t, "この方針で進めますか?"), time.Now())
	s.handleInput(proto.Message{SessionID: 2, Text: "1\r"})

	registerTestSession(s, 3, "claude")
	s.scanTranscriptApprovalMarkers(3, "claude", "transcript.jsonl", []agentChatMessage{
		{Role: "assistant", Kind: "text", Text: transcriptAssistantText()},
	}, false, time.Now())
	s.scanTranscriptApprovalMarkers(3, "claude", "transcript.jsonl", []agentChatMessage{
		{Role: "user", Kind: "text", Text: "1"},
	}, false, time.Now())

	messages := sent()
	for _, typ := range legacyApprovalMessageTypes {
		if n := countMessages(messages, typ); n != 0 {
			t.Errorf("%s を %d 件配信した, want 0", typ, n)
		}
	}
	for id, want := range map[int]int{1: 2, 2: 1, 3: 1} {
		if opens, closes := len(approvalStateOpens(messages, id)), len(approvalStateCloses(messages, id)); opens != want || closes != want {
			t.Errorf("session %d: 開く %d 件・閉じる %d 件, want どちらも %d 件（経路を通れていない）", id, opens, closes, want)
		}
	}
}

// awaitingApprovalWriter は AwaitingApproval へ書いてよい唯一の関数。
const awaitingApprovalWriter = "setAwaitingFromApprovalRecordLocked"

// 「保留中」（AwaitingApproval）を立てるのは、保留中の記録の有無から立てる 1 関数だけ。
// 以前は画面の申告（session_hint）とそのリースでも立ち、画面ごとに見え方が違うものが状態を
// 決めていた。代入と、定数を入れる composite literal を走査で探す。別の値からの写し
// （x.AwaitingApproval を session_update に載せる等）は書き込みではないので数えない。
// 「保留中」を運ぶフィールドは 2 つある。DisplayState は AwaitingUser だけでも "waiting" を返すので、
// AwaitingApproval だけを見ていると AwaitingUser への代入で同じ穴が開く。
// AwaitingUser は SessionActivity.Normalize が「AwaitingApproval なら立てる」ためにも書く
// （AwaitingApproval から導くだけなので、記録以外の出どころにはならない）。
// State へ "waiting" を直接書く形と、キーの無い SessionActivity のリテラルも同じ穴なので止める。
var awaitingFieldWriters = map[string][]struct{ file, fn string }{
	"AwaitingApproval": {{"approval_record.go", awaitingApprovalWriter}},
	"AwaitingUser":     {{"approval_record.go", awaitingApprovalWriter}, {"session_activity.go", "Normalize"}},
}

func TestAwaitingApprovalIsAssignedOnlyByApprovalRecord(t *testing.T) {
	allowed := map[string]int{}
	for _, file := range goSourceFiles(t) {
		fset := token.NewFileSet()
		parsed, err := parser.ParseFile(fset, file, nil, 0)
		if err != nil {
			t.Fatal(err)
		}
		for _, decl := range parsed.Decls {
			fn, _ := decl.(*ast.FuncDecl)
			inWriter := func(field string) bool {
				if fn == nil {
					return false
				}
				for _, w := range awaitingFieldWriters[field] {
					if fn.Name.Name == w.fn && filepath.Base(file) == w.file {
						return true
					}
				}
				return false
			}
			ast.Inspect(decl, func(n ast.Node) bool {
				switch node := n.(type) {
				case *ast.AssignStmt:
					for i, lhs := range node.Lhs {
						sel, ok := lhs.(*ast.SelectorExpr)
						if ok && sel.Sel.Name == "State" && i < len(node.Rhs) && isWaitingLiteral(node.Rhs[i]) {
							t.Errorf("%s: State に \"waiting\" を直接入れている（State は Activity.DisplayState() から出す）", fset.Position(node.Pos()))
						}
						if !ok || awaitingFieldWriters[sel.Sel.Name] == nil {
							continue
						}
						if inWriter(sel.Sel.Name) {
							allowed[sel.Sel.Name]++
							continue
						}
						t.Errorf("%s: %s へ許可した関数の外で代入している（「保留中」は記録からだけ出す）", fset.Position(node.Pos()), sel.Sel.Name)
					}
				case *ast.KeyValueExpr:
					key, ok := node.Key.(*ast.Ident)
					if ok && key.Name == "State" && isWaitingLiteral(node.Value) {
						t.Errorf("%s: State に \"waiting\" を直接入れている（State は Activity.DisplayState() から出す）", fset.Position(node.Pos()))
					}
					if !ok || awaitingFieldWriters[key.Name] == nil {
						return true
					}
					if sel, ok := node.Value.(*ast.SelectorExpr); ok && sel.Sel.Name == key.Name {
						return true // 別の値からの写し
					}
					t.Errorf("%s: %s に写し以外の値を入れている（「保留中」は記録からだけ出す）", fset.Position(node.Pos()), key.Name)
				case *ast.CompositeLit:
					// 位置指定の SessionActivity{...} は、キーが無いので上の検査をすり抜ける。
					ident, ok := node.Type.(*ast.Ident)
					if !ok || ident.Name != "SessionActivity" || len(node.Elts) == 0 {
						return true
					}
					if _, keyed := node.Elts[0].(*ast.KeyValueExpr); !keyed {
						t.Errorf("%s: SessionActivity を位置指定で作っている（「保留中」のフィールドをキー付きで書く）", fset.Position(node.Pos()))
					}
				}
				return true
			})
		}
	}
	for field, writers := range awaitingFieldWriters {
		if allowed[field] == 0 {
			t.Fatalf("%s への代入が許可した関数（%v）に無い。走査条件が実装とずれている", field, writers)
		}
	}
}

// isWaitingLiteral は式が文字列リテラル "waiting" かを返す。「保留中」の表示状態を記録を経ずに
// 直接書く形を止めるため。
func isWaitingLiteral(expr ast.Expr) bool {
	lit, ok := expr.(*ast.BasicLit)
	if !ok || lit.Kind != token.STRING {
		return false
	}
	value, err := strconv.Unquote(lit.Value)
	return err == nil && value == "waiting"
}
