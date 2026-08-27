package hub

// model_detect.go: server.go から分離した「PTY 出力からのモデル検出」の関数群。
//
// C4 追加分割 (plan_audit_score_s_promotion_2026-07-05.md): server.go の関心事別
// 分割の第四弾。以下 4 関数は「/model 変更検出」「起動バナーからの初期モデル
// 検出」「検出値のセッション反映」を扱う一塊で、他の関心事から明確に分離できる。
// 挙動は移動前と完全に同一・全て package-private・呼び出し元は変更なし。

import (
	"strings"

	"many-ai-cli/internal/proto"
)

func (s *Server) detectModelChange(id int, data []byte, cleanText string) {
	if !ptyChunkContainsAny(data, modelChangeTokens) {
		return
	}
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		return
	}
	provider := ses.Provider
	s.sessionsMu.Unlock()

	var match []string
	switch provider {
	case "claude":
		match = reSetModelTo.FindStringSubmatch(cleanText)
	case "codex":
		match = reCodexModelChanged.FindStringSubmatch(cleanText)
	default:
		return
	}
	if match == nil {
		return
	}
	newModel := strings.TrimSpace(match[1])
	if newModel == "" {
		return
	}
	// Claude は "Set model to Opus 5 with high effort" のように effort が同じ行に
	// 続くことがある。モデル名へ混ぜず effort として分離する（バナー経路と同じ扱い）。
	newEffort := ""
	if provider == "claude" {
		newModel, newEffort = splitClaudeModelEffort(newModel)
	}
	s.applyDetectedModel(id, provider, newModel, newEffort, false)
}

// splitClaudeModelEffort は "Opus 5 with high effort · Claude Pro" 形式の 1 行から
// モデル名と effort を分ける。プラン表記（" · Claude Pro"）は落とす。
func splitClaudeModelEffort(line string) (model, effort string) {
	rest := strings.TrimSpace(line)
	if before, _, found := strings.Cut(rest, "·"); found {
		rest = strings.TrimSpace(before)
	}
	if m := reClaudeBannerEffort.FindStringSubmatch(rest); m != nil {
		effort = m[1]
		rest = strings.TrimSpace(reClaudeBannerEffort.ReplaceAllString(rest, ""))
	}
	return rest, effort
}

// detectInitialModel は VT バッファのレンダリング済み行から起動バナーの
// モデル名と effort を抽出し、Model が空のセッションに反映する。
// /model 変更（detectModelChange）と違い既存値は上書きしない。
func (s *Server) detectInitialModel(id int, provider, cwd string, vtLines []string) {
	model, effort := extractBannerModel(provider, cwd, vtLines)
	if model == "" {
		return
	}
	s.applyDetectedModel(id, provider, model, effort, true)
}

// extractBannerModel は起動バナー / ステータス行からモデル名と reasoning effort を
// 抽出する（見つからなければ ""）。effort を取れるのは表記のある claude / copilot だけで、
// 他は "" を返す。cwd は cursor-agent のステータス行アンカーに使う。
func extractBannerModel(provider, cwd string, lines []string) (model, effort string) {
	switch provider {
	case "claude":
		// "Claude Code v<版>" 行の直後の非空行がモデル行。ロゴのアスキーアートは
		// 版で変わるので足場にしない（v2.1.246 で 2 行目の絵柄が変わり検出が止まった）。
		for i, line := range lines {
			if !reClaudeBannerVersion.MatchString(line) {
				continue
			}
			for j := i + 1; j < len(lines) && j <= i+2; j++ {
				rest := strings.TrimSpace(reClaudeBannerLogoPrefix.ReplaceAllString(lines[j], ""))
				if rest == "" {
					continue
				}
				name, eff := splitClaudeModelEffort(rest)
				if name != "" {
					return name, eff
				}
			}
		}
	case "codex":
		for _, line := range lines {
			m := reCodexBannerModel.FindStringSubmatch(line)
			if m == nil {
				continue
			}
			name := strings.TrimSpace(m[1])
			if name != "" && !strings.EqualFold(name, "loading") {
				return name, ""
			}
		}
	case "copilot":
		// 最下部の非空行（ステータス行）の右端セグメントを候補にする。
		// ラベルが無いため、モデル名らしさの検査で誤検出を防ぐ。
		for i := len(lines) - 1; i >= 0; i-- {
			line := strings.TrimSpace(lines[i])
			if line == "" {
				continue
			}
			segs := reCopilotStatusSplit.Split(line, -1)
			seg := strings.TrimSpace(segs[len(segs)-1])
			eff := ""
			if m := reCopilotEffortSuffix.FindStringSubmatch(seg); m != nil {
				eff = m[1]
				seg = strings.TrimSpace(reCopilotEffortSuffix.ReplaceAllString(seg, ""))
			}
			if seg == "Auto" || (len(seg) <= 40 && reCopilotModelLike.MatchString(seg)) {
				return seg, eff
			}
			return "", "" // 最下部の非空行のみ見る（それより上はステータス行ではない）
		}
	case "cursor-agent":
		if cwd == "" {
			return "", ""
		}
		// "<cwd> · <branch>" 行を探し、その直上の非空行をモデル名とみなす。
		for i, line := range lines {
			t := strings.TrimSpace(line)
			if !strings.HasPrefix(t, cwd+" · ") && t != cwd {
				continue
			}
			for j := i - 1; j >= 0; j-- {
				above := strings.TrimSpace(lines[j])
				if above == "" {
					continue
				}
				above = reCursorPercentSuffix.ReplaceAllString(above, "")
				// プロンプト残骸（"→ ..." 等）は除外
				if above != "" && !strings.ContainsAny(above, "→❯") && len(above) <= 60 {
					return above, ""
				}
				break
			}
		}
	}
	return "", ""
}

// applyDetectedModel はセッションの Model / Route / Effort を更新して session_update を
// broadcast する。onlyIfEmpty=true のときは Model 未設定のセッションのみ更新する
// （起動バナー検出が /model 変更や --model 指定を上書きしないため）。
// newEffort は検出できたときだけ渡す（空文字では既存の Effort を消さない）。
func (s *Server) applyDetectedModel(id int, provider, newModel, newEffort string, onlyIfEmpty bool) {
	newRoute := s.resolveRoute(provider, newModel)
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || (onlyIfEmpty && ses.Model != "") {
		s.sessionsMu.Unlock()
		return
	}
	changed := false
	if ses.Model != newModel {
		ses.Model = newModel
		ses.Route = newRoute
		changed = true
	}
	if newEffort != "" && ses.Effort != newEffort {
		ses.Effort = newEffort
		changed = true
	}
	if !changed {
		s.sessionsMu.Unlock()
		return
	}
	ses.initialModelScanDone = true
	update := proto.Message{
		Type:         "session_update",
		SessionID:    id,
		Provider:     ses.Provider,
		Display:      ses.Display,
		CWD:          ses.CWD,
		Branch:       ses.Branch,
		Label:        ses.Label,
		Model:        ses.Model,
		Effort:       ses.Effort,
		Route:        ses.Route,
		State:        ses.State,
		LastOutputAt: ses.LastOutputAt,
		FirstMessage: ses.FirstMessage,
		LastMessage:  ses.LastMessage,
	}
	s.sessionsMu.Unlock()
	s.broadcast(update)
}
