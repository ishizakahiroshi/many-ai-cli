package hub

import (
	"regexp"
	"strconv"
	"strings"

	"many-ai-cli/internal/sessionlog"
)

// 承認マーカーブロックの構造妥当性検査。
//
// 背景: docs/local/archive/v0.5.x/bugfix_codex-approval-marker-vt-wrap-corruption_2026-07-31.md
// Hub の VT ミラー（vt_buffer.go）が実端末と乖離すると、開始/終了マーカーは揃ったまま
// 内側の選択肢行だけが失われたブロックが抽出される。そのまま配信すると Web に
// 「選択肢が 3 から始まるパネル」「ボタン 1 個だけのパネル」が出て、ユーザーが
// 意図しない番号を送ってしまう。2026-07-31 に実測で確認した破損形は次の 3 つ。
//
//	選択肢 [3 4]（Q1 の選択肢が丸ごと欠落）
//	選択肢 [4]（見出しも選択肢 1 個も欠落）
//	ブロック内に開始マーカーが 2 個（複数世代の再描画が融合）
//
// 検査の根拠は ~/.many-ai-cli/approval-rules.md の書式。同ファイルは
// 「ブロック全体の通し番号」と「質問ごとに 1 から振り直し」の両方を許容するが、
// どちらの書式でも最初の選択肢番号は必ず 1 になる。1 以外で始まるのは欠落の証拠。

// マーカー文字列はリテラル連結で組み立てる。連続した実文字列をソースへ置くと、
// 本リポジトリを AI エージェント経由で編集するときに Web ダッシュボードの
// hub-marker-filter が誤爆し、以降の出力が UI から消える（approval-rules.md 参照）。
const (
	approvalMarkerOpen  = "[" + "MANY-AI-CLI" + "]"
	approvalMarkerClose = "[/" + "MANY-AI-CLI" + "]"
)

var (
	// 選択肢行: 行頭（インデント可）の「数字 + . + 空白 + 本文」。
	// `N. User specifies` / `N. 自由入力` は数字でないので対象外＝自由入力肢は数えない。
	approvalOptionNumRe = regexp.MustCompile(`^[ \t]*([0-9]{1,2})\.[ \t]+\S`)

	// 質問見出し。approval-rules.md の「Q + 連番」。番号の重複検査を質問単位に区切る用途。
	approvalHeadingLineRe = regexp.MustCompile(`^[ \t]*[QＱ][ \t]*[0-9]{1,2}(?:[ \t]|$)`)

	// Y/N 形式は選択肢行を持たず、パーサ側が 1/0 を合成する。番号検査の対象外。
	approvalYesNoFormRe = regexp.MustCompile(`[（(]\s*[YＹ]\s*[:：]\s*1\s*[/／]\s*[NＮ]\s*[:：]\s*0\s*[）)]`)

	// 罫線（box drawing）の連続。approval-rules.md はマーカーブロック内での罫線・表組みを
	// 明示的に禁じているため、ブロック内に現れる罫線は AI の出力ではなく、TUI のコンポーザ枠が
	// 再描画で本文へ重なった証拠になる。実履歴 731 ブロック（claude / codex）で AI 由来の
	// 罫線は 0 件、検出された 1 件は本症状と同型の重なりだった。
	// しきい値 3 は「10─20」のような範囲表記で誤爆させないための余裕。
	approvalBoxRuleRe = regexp.MustCompile(`[\x{2500}-\x{257F}]{3,}`)
)

// classifyApprovalMarkerBlock は構造が壊れていれば理由を返す。正常なら空文字を返す。
//
// 誤検出（正常なブロックを壊れていると判定する）は「承認が無音で出ない」事故になるため、
// 判定は保守的に倒す。曖昧なケースは正常扱いにする。
func classifyApprovalMarkerBlock(block string) string {
	clean := sessionlog.StripANSI(block)

	// 複数世代の再描画が融合したブロック。非貪欲マッチでも
	// OPEN … OPEN … CLOSE の並びでは内側の OPEN がブロックに含まれる。
	if strings.Count(clean, approvalMarkerOpen) > 1 || strings.Count(clean, approvalMarkerClose) > 1 {
		return "marker_leak"
	}

	// TUI コンポーザの枠線が本文へ重なった再描画。選択肢ラベルの末尾だけが罫線で
	// 上書きされてもマーカーの対と番号構造は保たれるため、下の番号検査では捕まらない。
	// Y/N 形式の早期 return より前に置くこと（Y/N ブロックも同じ壊れ方をする）。
	if approvalBoxRuleRe.MatchString(clean) {
		return "box_rule"
	}

	if approvalYesNoFormRe.MatchString(clean) {
		return ""
	}

	// 選択肢番号を質問見出しで区切ってグループ化する。
	// approval-rules.md が禁じているのは「同一質問内の重複」だけで、質問をまたいだ
	// 番号の再利用（Q1 が 1,2,3 / Q2 が 3,4,5）は許容されている。平坦なリストで
	// 重複を見ると、その正当な書式を破損と誤判定する（実履歴 4,545 ブロック中 3 件で発生）。
	var groups [][]int
	var cur []int
	for _, raw := range strings.Split(clean, "\n") {
		if approvalHeadingLineRe.MatchString(raw) {
			if len(cur) > 0 {
				groups = append(groups, cur)
				cur = nil
			}
			continue
		}
		m := approvalOptionNumRe.FindStringSubmatch(raw)
		if m == nil {
			continue
		}
		n, err := strconv.Atoi(m[1])
		if err != nil {
			continue
		}
		cur = append(cur, n)
	}
	if len(cur) > 0 {
		groups = append(groups, cur)
	}

	// 選択肢行が 1 つも無い形式（説明だけのブロック・自由入力のみ等）は判定しない。
	if len(groups) == 0 {
		return ""
	}

	// 選択肢番号 1 がどこにも無いのは、先頭の質問の選択肢が丸ごと失われた証拠。
	// 「先頭が 1」ではなく「1 を含む」で判定するのが要点 — 前置き（経緯）の地の文に
	// 行頭数字が混じることがあり、先頭一致だと正常なブロックを弾いてしまう。
	has1 := false
	for _, g := range groups {
		for _, n := range g {
			if n == 1 {
				has1 = true
				break
			}
		}
	}
	if !has1 {
		return "option_start"
	}

	// 同一質問内の重複だけを破損とみなす。前置き由来の擬似番号を巻き込まないよう、
	// 各グループの最初の 1 以降だけを見る。
	for _, g := range groups {
		i1 := -1
		for i, n := range g {
			if n == 1 {
				i1 = i
				break
			}
		}
		if i1 < 0 {
			continue
		}
		seen := make(map[int]bool, len(g))
		for _, n := range g[i1:] {
			if seen[n] {
				return "duplicate_option"
			}
			seen[n] = true
		}
	}

	return ""
}

// shortSig はログ用にシグネチャを先頭 10 文字へ短縮する。
func shortSig(sig string) string {
	if len(sig) <= 10 {
		return sig
	}
	return sig[:10]
}
