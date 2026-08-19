package hub

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// approvalSuppressionFields は「この承認はもう回答済みか」を決める session の
// state の全量である。v0.7 でこの役目は 3 本（approvalConsumedSig の TTL 方式 /
// answeredMarkerSigs のブロック全文ハッシュ / approvalQuestionKey）から
// candidateKey + sourceEpoch の 1 本へ統合された。3 者はそれぞれ「同じ質問とは
// 何か」の定義が違い、その食い違いそのものが症状だった（タイマー方式は再描画中
// に失効して回答済みを再表示し、ブロック全文方式は本当の再質問まで永久に埋めた）。
//
// approvalConsumedCarried はこの一覧にあるが抑止 state ではない。持ち越しを 1 回
// に制限するだけで、単独で候補を抑止する力を持たない（2026-08-19 追加）。
var approvalSuppressionFields = map[string]string{
	"approvalSourceEpoch":            "候補の世代。live prompt の境界でのみ進む",
	"approvalEpochPending":           "直前の消費が現世代のものか",
	"approvalConsumedCandidateKey":   "消費済み候補の identity",
	"approvalConsumedCandidateShape": "消費済み候補の shape（identity の材料・表示用）",
	"approvalConsumedEpoch":          "消費が起きた世代",
	"approvalConsumedCarried":        "持ち越しを 1 回に制限する境界フラグ（抑止 state ではない）",

	// 以下 3 本は「いま画面に出 している候補」であって「回答済みか」ではない。
	// approval_marker.go:129 で再 broadcast を抑えるのに使うが、判定に使う同一性は
	// candidateKey + sourceEpoch とまったく同じもので、2 つ目の定義を持ち込んでいない。
	// v0.7 で撤去した 3 本が問題だったのは、それぞれ別の同一性を持っていたためである。
	"approvalMarkerCandidateKey":   "いま表示中のマーカー候補の identity（同じ定義を使う）",
	"approvalMarkerCandidateShape": "いま表示中のマーカー候補の shape",
	"approvalMarkerSourceEpoch":    "いま表示中のマーカー候補の世代",
}

// TestApprovalSuppressionStateIsSingleSource は、承認の抑止 state が増えていない
// ことをソース走査で固定する。
//
// approval_identity.go 冒頭のルール（正本）の機械的な裏づけ。承認の誤表示は
// 踏むたびに「もう 1 本抑止を足す」が最短の対処に見えるが、v0.7 の 3 本並立が
// まさにそれで作られた。直すのは candidateKey の作り方か sourceEpoch の進み方で
// あって、新しい state ではない。
//
// このテストが落ちたときの正しい対処は、フィールドを一覧へ足して通すことではない。
// 足そうとしている state が本当に「同じ質問とは何か」の 2 つ目の定義になっていない
// かを先に確かめること。なっているなら設計をやり直す。
func TestApprovalSuppressionStateIsSingleSource(t *testing.T) {
	body, err := os.ReadFile("server.go")
	if err != nil {
		t.Fatal(err)
	}
	var found []string
	for _, line := range strings.Split(string(body), "\n") {
		trimmed := strings.TrimSpace(line)
		if strings.HasPrefix(trimmed, "//") {
			continue
		}
		fields := strings.Fields(trimmed)
		if len(fields) < 2 {
			continue
		}
		name := fields[0]
		if !strings.HasPrefix(name, "approval") && !strings.HasPrefix(name, "answered") {
			continue
		}
		// 宣言行だけを見る（型が続く形）。代入や呼び出しは対象外。
		if strings.ContainsAny(name, "(){}[]=.,:") {
			continue
		}
		switch fields[1] {
		case "string", "bool", "uint64", "int", "time.Time":
		default:
			continue
		}
		found = append(found, name)
	}
	if len(found) == 0 {
		t.Fatal("approval 系フィールドを 1 つも拾えなかった。走査条件が実装とずれている")
	}
	for _, name := range found {
		if _, ok := approvalSuppressionFields[name]; ok {
			continue
		}
		// 抑止に関わらない approval 系フィールド（候補の保持・署名など）は
		// 名前で除外する。ここを広げるときは本当に抑止でないかを確かめること。
		if strings.Contains(name, "Candidate") || strings.Contains(name, "Consumed") ||
			strings.Contains(name, "Epoch") || strings.Contains(name, "Answered") {
			t.Errorf("承認の抑止に関わる新しい state %q が増えている。"+
				"approval_identity.go 冒頭のルールを読み、"+
				"candidateKey の作り方か sourceEpoch の進み方で直せないかを先に検討すること", name)
		}
	}
}

// TestApprovalIdentityRuleIsDocumentedInSource は、承認の同一性ルールの本文が
// コード側に置かれたままであることを確かめる。
//
// このルールは以前 CLAUDE.md に 23 行の節として置かれていたが、常時ロードされる
// ファイルが事故のたびに太る原因になっていた（2026-08-19 の再編）。承認コードを
// 直す担当は必ずこのファイルを開くので、本文はここにあるほうが確実に届く。
// CLAUDE.md 側は索引の 1 行だけを持つ。
func TestApprovalIdentityRuleIsDocumentedInSource(t *testing.T) {
	body, err := os.ReadFile(filepath.Join("approval_identity.go"))
	if err != nil {
		t.Fatal(err)
	}
	text := string(body)
	for _, needle := range []string{
		"candidateKey",
		"sourceEpoch",
		"抑止",
	} {
		if !strings.Contains(text, needle) {
			t.Errorf("approval_identity.go に同一性ルールの説明 %q が無い。"+
				"CLAUDE.md へ書き戻さず、正本であるこのファイルに置くこと", needle)
		}
	}
}
