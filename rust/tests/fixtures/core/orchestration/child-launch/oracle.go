// Extracted verbatim functions from Go oracle 21d0bc7935a2c4696fb89ccff2e324157a528c2d.
package main
import ("encoding/json"; "os"; "path/filepath"; "regexp"; "strings")
var safeOrchestrationToken = regexp.MustCompile(`[^a-zA-Z0-9._-]+`)
func safeToken(value string) string {
	value = safeOrchestrationToken.ReplaceAllString(strings.TrimSpace(value), "-")
	value = strings.Trim(value, "-_.")
	if value == "" {
		return "item"
	}
	if len(value) > 80 {
		value = value[:80]
	}
	return value
}

func sanitizeRole(role string) string {
	role = strings.ToLower(strings.TrimSpace(role))
	role = safeToken(role)
	return strings.Trim(role, "-_.")
}

func sanitizeInjectText(s string) string {
	s = strings.ReplaceAll(s, "\r\n", "\n")
	s = strings.ReplaceAll(s, "\r", "\n")
	return strings.Map(func(r rune) rune {
		if (r < 0x20 && r != '\t' && r != '\n') || r == 0x7f {
			return -1
		}
		return r
	}, s)
}

func childProgressPathFor(boardPath, id string) string {
	return filepath.Join(filepath.Dir(boardPath), "child-"+id+".md")
}

func buildChildInitialPromptFor(base, boardPath, role, branch, id string) string {
	var b strings.Builder
	b.WriteString("You are an orchestration child session.\n")
	b.WriteString("Role: " + role + "\n")
	b.WriteString("Session ID: " + id + "\n")
	b.WriteString("Shared board (read-only for you): " + boardPath + "\n")
	b.WriteString("Your progress file (write here): " + childProgressPathFor(boardPath, id) + "\n")
	if branch != "" {
		b.WriteString("Worktree branch: " + branch + "\n")
	}
	// 進捗・DONE は子専用ファイルへ。board.md は conductor の指示・全体状況の読み取り専用に
	// することで、共有 board への同時書き込み競合と記帳名義ゆれを避ける（C4）。
	b.WriteString("Read the board before acting; the conductor posts instructions there. Write your progress ONLY to your progress file (create it on first write), as `## " + role + " session=" + id + " <RFC3339 time>` sections, each including a `status: running|blocked|done|failed` line. If you need an answer before continuing, include `status: blocked` and append `## QUESTION " + role + " session=" + id + "`; the conductor will answer with orchestrate send. When complete, append `## DONE " + role + " session=" + id + "` (or the explicit success form `## SUCCESS " + role + " session=" + id + "`) and a concise summary to your progress file. Do not write to the shared board.\n\n")
	// base はユーザー・conductor 由来のフリーテキスト。BEL/ESC 等の C0 制御文字が
	// 混入していると PTY 経由で子セッションの端末エコー・Hub UI レンダリングに
	// エスケープシーケンス（タイトル詐称・画面クリア等）を注入できてしまうため
	// git_common.go の sanitizeCommitMessage と同型のフィルタで除去する。
	b.WriteString(sanitizeInjectText(base))
	return b.String()
}
type input struct { Name, Base, Board, Role, Branch, ID, Token string }
type output struct { Input input; Prompt, Sanitized, Safe, Role string }
func main() {
 inputs := []input{
  {Name:"basic", Base:"review the diff", Board:"/synthetic/run/board.md", Role:"review", Branch:"orch/run/review", ID:"42", Token:"  --Run:β / Review__  "},
  {Name:"controls", Base:"a\r\nb\rc\x00\x07\x1b[31m\t日本\x7f\n", Board:"/synthetic/run/board.md", Role:"implementation", ID:"{{many-ai-cli:session-id}}", Token:"...."},
  {Name:"long-token", Base:"\nhi\n", Board:"board.md", Role:"review", ID:"7", Token:strings.Repeat("a",79)+" / b"},
  {Name:"unicode-role", Base:"done", Board:"/synthetic/sp ace/board.md", Role:"  Ｒｅｖｉｅｗ 💡  ", ID:"9", Token:"a💡βc"},
 }
 outputs := make([]output,0,len(inputs))
 for _, in := range inputs { outputs=append(outputs,output{Input:in,Prompt:buildChildInitialPromptFor(in.Base,in.Board,in.Role,in.Branch,in.ID),Sanitized:sanitizeInjectText(in.Base),Safe:safeToken(in.Token),Role:sanitizeRole(in.Role)}) }
 encoder:=json.NewEncoder(os.Stdout); encoder.SetIndent("","  "); if err:=encoder.Encode(outputs); err!=nil { panic(err) }
}
