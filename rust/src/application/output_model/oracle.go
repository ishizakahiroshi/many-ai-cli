package main
import ("encoding/json";"os";"regexp";"strings")
var (
	// reClaudeBannerVersion は起動バナー 1 行目（ロゴ + "Claude Code v2.1.246"）。
	// モデル行はこの次の行に来る。
	reClaudeBannerVersion = regexp.MustCompile(`Claude Code\s+v\d`)
	// reClaudeBannerLogoPrefix は行頭のロゴ（Block Elements）と空白。
	reClaudeBannerLogoPrefix = regexp.MustCompile(`^[\s\x{2580}-\x{259F}]+`)
	reClaudeBannerEffort     = regexp.MustCompile(`\s+with\s+(\S+)\s+effort$`)
	reCodexBannerModel       = regexp.MustCompile(`model:\s+(.+?)\s+/model to change`)
	reCopilotStatusSplit     = regexp.MustCompile(`\s{3,}`)
	reCopilotEffortSuffix    = regexp.MustCompile(`\s+·\s+(low|medium|high|xhigh)$`)
	reCopilotModelLike       = regexp.MustCompile(`^[A-Za-z][\w.\- ()]*\d`)
	reCursorPercentSuffix    = regexp.MustCompile(`\s+·\s+\d+(?:\.\d+)?%$`)
)

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

type Input struct{ Provider string `json:"provider"`; Cwd string `json:"cwd"`; Lines []string `json:"lines"` }
type Output struct{ Model string `json:"model"`; Effort string `json:"effort"` }
func main(){var in []Input;if err:=json.NewDecoder(os.Stdin).Decode(&in);err!=nil{panic(err)};out:=make([]Output,0,len(in));for _,v:=range in{m,e:=extractBannerModel(v.Provider,v.Cwd,v.Lines);out=append(out,Output{m,e})};if err:=json.NewEncoder(os.Stdout).Encode(out);err!=nil{panic(err)}}