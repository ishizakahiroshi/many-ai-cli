package sessionlog

import (
	"bufio"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"regexp"
	"strings"
)

var (
	controlRE = regexp.MustCompile(`[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]`)
	blankRun  = regexp.MustCompile(`\n{3,}`)

	// spinnerAnimRe は Claude/Codex の「思考中」スピナーが描く星形 dingbat
	// （✢✳✶✷✻✽ 等 U+2722–U+273F）と braille 進捗グリフ（⠋⠙ 等）。会話本文には
	// まず現れない記号に絞り、✓✗ など普通に使われる dingbat を誤検出しないようにする。
	spinnerAnimRe = regexp.MustCompile(`[\x{2722}-\x{273F}\x{2800}-\x{28FF}]`)
)

// IsThinkingNoiseLine は 1 行が AI CLI の「思考中」ステータス／スピナー再描画
// フレームかどうかを判定する。PTY を ANSI 除去しただけでは、思考中行
// （"✳ Imploring… (12s · ↑3.2k tokens · esc to interrupt)" 等）の再描画フレームが
// 大量に連結して残るため、会話本文と区別してまるごと落とすのに使う。
// 誤検出を避けるため、強いシグネチャ（"esc to interrupt"・トークンバー・
// "thinking"+スピナーグリフ・グリフ密集）に限定する。
func IsThinkingNoiseLine(line string) bool {
	t := strings.TrimSpace(line)
	if t == "" {
		return false
	}
	lower := strings.ToLower(t)
	// 1) ステータスフッター（Claude/Codex とも "esc to interrupt" を必ず表示する）
	if strings.Contains(lower, "esc to interrupt") {
		return true
	}
	// 2) モード切替ヒント行（"auto mode on (shift+tab to cycle)" 等）
	if strings.Contains(lower, "shift+tab to cycle") || strings.Contains(lower, "shift + tab to cycle") {
		return true
	}
	// 3) トークンバー "↑111.0k ↓764"（U+2191/U+2193 の両方を含む行）
	if strings.Contains(t, "↑") && strings.Contains(t, "↓") {
		return true
	}
	// 4) "thinking" + スピナーグリフ（思考アニメの再描画フレーム）
	if strings.Contains(lower, "thinking") && spinnerAnimRe.MatchString(t) {
		return true
	}
	// 5) スピナーグリフが複数現れる断片（"Imp·rmpovri✶osviisng✶..." 等）。
	//    対象は星形 dingbat と braille に限るため、本文に 2 個以上並ぶのは実質スピナーのみ。
	if len(spinnerAnimRe.FindAllString(t, 2)) >= 2 {
		return true
	}
	return false
}

type transcriptEvent struct {
	TS                string `json:"ts"`
	Type              string `json:"type"`
	SessionID         int    `json:"session_id"`
	Provider          string `json:"provider"`
	CWD               string `json:"cwd"`
	Branch            string `json:"branch"`
	Model             string `json:"model"`
	Shell             string `json:"shell"`
	PID               int    `json:"pid"`
	Filename          string `json:"filename"`
	Path              string `json:"path"`
	Text              string `json:"text"`
	DataB64           string `json:"data_b64"`
	State             string `json:"state"`
	ExitCode          int    `json:"exit_code"`
	CombinedHasInject bool   `json:"combined_has_inject"`
}

// CleanVisibleText keeps terminal-visible text and removes escape/control bytes
// that make raw PTY history hard to read in plain editors.
func CleanVisibleText(s string) string {
	s = StripANSI(s)
	s = strings.ReplaceAll(s, "\r\n", "\n")
	s = strings.ReplaceAll(s, "\r", "\n")
	s = controlRE.ReplaceAllString(s, "")
	return s
}

func WriteTranscriptFile(jsonlPath, outPath string) error {
	in, err := os.Open(jsonlPath)
	if err != nil {
		return err
	}
	defer in.Close()

	out, err := os.OpenFile(outPath, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, PrivateFileMode)
	if err != nil {
		return err
	}
	defer out.Close()

	return WriteTranscript(in, out)
}

func WriteTranscript(r io.Reader, w io.Writer) error {
	sc := bufio.NewScanner(r)
	sc.Buffer(make([]byte, 64*1024), 8*1024*1024)

	var output strings.Builder
	var lastOutput string

	flushOutput := func() error {
		text := normalizeTranscriptBlock(output.String())
		output.Reset()
		if text == "" || text == lastOutput {
			return nil
		}
		lastOutput = text
		_, err := fmt.Fprintf(w, "\n[output]\n%s\n", text)
		return err
	}

	for sc.Scan() {
		var ev transcriptEvent
		if err := json.Unmarshal(sc.Bytes(), &ev); err != nil {
			return fmt.Errorf("parse jsonl: %w", err)
		}
		switch ev.Type {
		case "pty_output":
			text := ev.Text
			if text == "" && ev.DataB64 != "" {
				if b, err := base64.StdEncoding.DecodeString(ev.DataB64); err == nil {
					text = string(b)
				}
			}
			output.WriteString(CleanVisibleText(text))
		case "session_start":
			if err := flushOutput(); err != nil {
				return err
			}
			fmt.Fprintf(w, "[%s] session_start #%d %s pid=%d\n", ev.TS, ev.SessionID, ev.Provider, ev.PID)
			if ev.CWD != "" {
				fmt.Fprintf(w, "cwd: %s\n", ev.CWD)
			}
			if ev.Branch != "" {
				fmt.Fprintf(w, "branch: %s\n", ev.Branch)
			}
			if ev.Model != "" {
				fmt.Fprintf(w, "model: %s\n", ev.Model)
			}
			if ev.Shell != "" {
				fmt.Fprintf(w, "shell: %s\n", ev.Shell)
			}
		case "attach":
			if err := flushOutput(); err != nil {
				return err
			}
			fmt.Fprintf(w, "\n[%s] attach %s\n%s\n", ev.TS, ev.Filename, ev.Path)
		case "user_input":
			if err := flushOutput(); err != nil {
				return err
			}
			text := strings.TrimSpace(CleanVisibleText(ev.Text))
			if ev.CombinedHasInject {
				fmt.Fprintf(w, "\n[%s] user_input + attachment\n> %s\n", ev.TS, text)
			} else {
				fmt.Fprintf(w, "\n[%s] user_input\n> %s\n", ev.TS, text)
			}
		case "session_end":
			if err := flushOutput(); err != nil {
				return err
			}
			fmt.Fprintf(w, "\n[%s] session_end state=%s exit_code=%d\n", ev.TS, ev.State, ev.ExitCode)
		default:
			if err := flushOutput(); err != nil {
				return err
			}
			fmt.Fprintf(w, "\n[%s] %s\n", ev.TS, ev.Type)
		}
	}
	if err := sc.Err(); err != nil {
		return err
	}
	return flushOutput()
}

func normalizeTranscriptBlock(s string) string {
	s = CleanVisibleText(s)
	lines := strings.Split(s, "\n")
	cleaned := make([]string, 0, len(lines))
	prev := ""
	for _, line := range lines {
		line = strings.TrimRight(line, " \t")
		if strings.TrimSpace(line) == "" {
			if prev != "" {
				cleaned = append(cleaned, "")
				prev = ""
			}
			continue
		}
		trimmed := strings.TrimSpace(line)
		if isSpinnerLine(trimmed) || trimmed == prev {
			continue
		}
		cleaned = append(cleaned, line)
		prev = trimmed
	}
	text := strings.TrimSpace(strings.Join(cleaned, "\n"))
	return blankRun.ReplaceAllString(text, "\n\n")
}

func isSpinnerLine(s string) bool {
	if len([]rune(s)) <= 2 {
		return true
	}
	switch s {
	case "Boot", "Boo", "Bo", "Thinking", "Working":
		return true
	}
	return IsThinkingNoiseLine(s)
}
