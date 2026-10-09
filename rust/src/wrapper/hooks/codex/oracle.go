package main
import("encoding/json";"fmt";"os";"regexp";"strings")
const hubTokenEnvName="MANY_AI_CLI_HUB_TOKEN"
const (
	usageHookBlockStart = "# many-ai-cli:usage-hook-start"
	usageHookBlockEnd   = "# many-ai-cli:usage-hook-end"

	// 旧名 any-ai-cli 時代（v0.3.x 以前のバイナリ）が注入したブロックの検出・除去用。
	// 新規注入には使わない。
	legacyUsageHookBlockStart = "# any-ai-cli:usage-hook-start"
	legacyUsageHookBlockEnd   = "# any-ai-cli:usage-hook-end"
)
type UsageHookParams struct {
	HubURL    string
	Token     string
	SessionID int
	ExePath   string // many-ai-cli バイナリのフルパス（os.Executable() で解決済み）
}
func toShellPath(p string) string {
	// filepath.ToSlash は実行時 OS の PathSeparator しか変換しないため、
	// Linux CI 上で Windows パスを扱うテストでは `\` が残る。POSIX シェル
	// 用フックは常に `/` 区切りなので、無条件で `\` を `/` に置換する。
	return strings.ReplaceAll(p, `\`, "/")
}
func usageHookQuotePOSIX(s string) string {
	return "'" + strings.ReplaceAll(s, "'", `'\''`) + "'"
}
func codexStopHookBlock(p UsageHookParams) string {
	// exe パスのみクォートする。HubURL（ポート番号のみ）と Token（hex）は型上
	// シェルメタ文字を持てないためクォート不要だが、exe パスはユーザーの配置場所
	// 次第でスペースを含み得るので語分割を防ぐ。
	cmd := fmt.Sprintf("%s=%s %s usage-relay --provider codex --hub %s --session %d",
		hubTokenEnvName, p.Token, usageHookQuotePOSIX(toShellPath(p.ExePath)), p.HubURL, p.SessionID)
	return strings.Join([]string{
		usageHookBlockStart,
		"[[hooks.Stop]]",
		fmt.Sprintf("command = %q", cmd),
		usageHookBlockEnd,
		"",
	}, "\n")
}
func inject(content string,p UsageHookParams) string {
		// 旧名 any-ai-cli マーカーのブロックが残っていれば先に除去する
		// （残したまま新マーカーで追記すると Stop フックが二重登録になる）。
		if strings.Contains(content, legacyUsageHookBlockStart) {
			legacyRe := regexp.MustCompile(`(?s)\n?` + regexp.QuoteMeta(legacyUsageHookBlockStart) + `.*?` + regexp.QuoteMeta(legacyUsageHookBlockEnd) + `\n?`)
			content = legacyRe.ReplaceAllString(content, "")
		}

		// already 注入済みかどうか確認。
		if strings.Contains(content, usageHookBlockStart) {
			// 注入済み: コマンドだけ更新（セッション ID が変わる場合を考慮）。
			// ReplaceAllLiteralString を使う。ReplaceAllString だと newBlock 中の
			// `$name` / `${name}` が Regexp.Expand ルールで解釈され、many-ai-cli の
			// 実行ファイルパスに `$` を含むディレクトリ（例:
			// `C:\tools\alice\$portable\many-ai-cli.exe`）があると、対応するキャプチャ
			// グループが無いため無言で消える（TOML 上は構文的に壊れないので気づけない）。
			newBlock := codexStopHookBlock(p)
			blockRe := regexp.MustCompile(`(?s)` + regexp.QuoteMeta(usageHookBlockStart) + `.*?` + regexp.QuoteMeta(usageHookBlockEnd) + `\n?`)
			content = blockRe.ReplaceAllLiteralString(content, newBlock)
			return content
		}

		// 末尾に追記。
		if !strings.HasSuffix(content, "\n") && len(content) > 0 {
			content += "\n"
		}
		content += "\n" + codexStopHookBlock(p)

		return content
}
func remove(content string) string {

		if !strings.Contains(content, usageHookBlockStart) && !strings.Contains(content, legacyUsageHookBlockStart) {
			return content
		}

		// 旧名 any-ai-cli マーカーのブロックも除去対象（旧バイナリからの移行）。
		blockRe := regexp.MustCompile(`(?s)\n?(?:` +
			regexp.QuoteMeta(usageHookBlockStart) + `.*?` + regexp.QuoteMeta(usageHookBlockEnd) + `|` +
			regexp.QuoteMeta(legacyUsageHookBlockStart) + `.*?` + regexp.QuoteMeta(legacyUsageHookBlockEnd) + `)\n?`)
		newContent := blockRe.ReplaceAllString(content, "")
		if newContent == content {
			return content
		}

		return newContent
}
type Case struct{Name,Content,Exe string}
type Result struct{Name,Block,Injected,Removed string}
func main(){var cases []Case;if err:=json.NewDecoder(os.Stdin).Decode(&cases);err!=nil{panic(err)};results:=[]Result{}
for _,c:=range cases{p:=UsageHookParams{HubURL:"http://127.0.0.1:49686",Token:"synthetic_test_token",SessionID:7,ExePath:c.Exe};results=append(results,Result{c.Name,codexStopHookBlock(p),inject(c.Content,p),remove(c.Content)})};json.NewEncoder(os.Stdout).Encode(results)}
