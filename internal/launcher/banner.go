package launcher

import (
	"fmt"
	"strings"
)

const repositoryURL = "https://github.com/ishizakahiroshi/many-ai-cli"

const (
	ansiReset        = "\x1b[0m"
	ansiBold         = "\x1b[1m"
	ansiReverse      = "\x1b[7m"
	ansiBrightOrange = "\x1b[38;5;208m"
	ansiLogoFill     = "\x1b[97m"
	ansiLogoOutline  = "\x1b[38;5;226m"
)

// ロゴと配色は internal/hub/banner.go と同一デザイン。hub パッケージを import
// すると HTTP サーバ一式までランチャー exe に取り込まれるため、描画部品だけを
// 複製している（デザイン変更時は両方を揃えること）。
var unicodeLogoLines = []string{
	" █████╗ ███╗   ██╗██╗   ██╗       █████╗ ██╗",
	"██╔══██╗████╗  ██║╚██╗ ██╔╝      ██╔══██╗██║",
	"███████║██╔██╗ ██║ ╚████╔╝ █████╗███████║██║",
	"██╔══██║██║╚██╗██║  ╚██╔╝  ╚════╝██╔══██║██║",
	"██║  ██║██║ ╚████║   ██║         ██║  ██║██║",
	"╚═╝  ╚═╝╚═╝  ╚═══╝   ╚═╝         ╚═╝  ╚═╝╚═╝",
}

// StartupBanner はランチャー起動直後に表示するスプラッシュを返す。
// Hub（serve）の startupBanner と同じ見た目で、サブタイトルだけランチャー用。
func StartupBanner(version string) string {
	logoLines := unicodeLogoLines
	colorize := colorizeLogoLine
	lines := make([]string, 0, len(logoLines)+5)
	for _, line := range logoLines {
		lines = append(lines, colorize(line))
	}
	lines = append(lines,
		"",
		fmt.Sprintf("Connection launcher (WSL / SSH) %s", formatVersionLabel(version)),
		"Runtime: Windows",
		fmt.Sprintf("GitHub: %s", repositoryURL),
		"",
	)
	return strings.Join(lines, "\n") + "\n"
}

// CloseBehaviorNotice は「このウィンドウを閉じると何が起きるか」を接続方式
// ごとに説明する注意書きを返す。接続開始時に表示する。
//
//   - ssh tunnel: トンネルだけが切れる。接続先の常駐 Hub・セッションは継続
//   - ssh serve : リモートで起動した Hub ごと終了（セッションも終了）
//   - wsl       : WSL 側で起動した Hub ごと終了（セッションも終了）
func CloseBehaviorNotice(p Profile) string {
	warning := ansiBold + ansiReverse + ansiBrightOrange +
		" WARNING: This window is the connection itself. Do not close it while in use. " + ansiReset
	var detail string
	if p.Type == ProfileTypeSSH && p.Mode == SSHModeTunnel {
		detail = "Closing this window disconnects only the SSH tunnel. The persistent Hub and sessions on the target keep running. Launch the connector again to reconnect and continue where you left off."
	} else {
		detail = "Closing this window also stops the Hub started on the target, including any running sessions."
	}
	return warning + "\n" + detail + "\n"
}

func colorizeLogoLine(line string) string {
	var b strings.Builder
	current := ""
	for _, r := range line {
		var next string
		switch r {
		case '█':
			next = ansiLogoFill
		case '╗', '╔', '╝', '╚', '║', '═':
			next = ansiLogoOutline
		default:
			next = ""
		}
		if next != current {
			if current != "" {
				b.WriteString(ansiReset)
			}
			if next != "" {
				b.WriteString(next)
			}
			current = next
		}
		b.WriteRune(r)
	}
	if current != "" {
		b.WriteString(ansiReset)
	}
	return b.String()
}

func formatVersionLabel(version string) string {
	v := strings.TrimSpace(version)
	if v == "" {
		return "dev"
	}
	if v == "dev" || strings.HasPrefix(v, "v") {
		return v
	}
	return "v" + v
}
