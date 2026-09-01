package wrapper

import (
	"strings"

	"many-ai-cli/internal/config"
)

const (
	// noColorEnvName は「色を出すな」という利用者の意思表示（https://no-color.org）。
	// 値ではなく存在で判定する実装があるため、落とすときはキーごと落とす。
	noColorEnvName = "NO_COLOR"
	// forceColorEnvName / clicolorForceEnvName は「色を出せ」の強制指定。
	// 色を出さない方針のときは、継承したこれらを外さないと NO_COLOR より優先される
	// 実装がある（Claude Code 同梱の bun ランタイムは FORCE_COLOR を先に見る）。
	forceColorEnvName    = "FORCE_COLOR"
	clicolorForceEnvName = "CLICOLOR_FORCE"

	// envTerm / envColorterm は端末の能力の訂正。Hub のペインは xterm.js なので、
	// 起動元の端末が dumb でも 256 色・truecolor を出せる。色方針に関わらず常に渡す。
	envTerm      = "TERM=xterm-256color"
	envColorterm = "COLORTERM=truecolor"

	// envForceColor の 3 は色深度のレベル指定で 24bit（truecolor）を意味する。
	// 1=16 色 / 2=256 色 / 3=24bit で、Node・bun・chalk 系が共通で解釈する。
	envForceColor = forceColorEnvName + "=3"
	// envClicolorForce は Rust 系 CLI（anstream / anstyle-query 等）が見る強制フラグ。
	// レベルではなく on/off で 1 が on。
	envClicolorForce = clicolorForceEnvName + "=1"

	// off のとき渡す組。NO_COLOR だけだと FORCE_COLOR を先に見る実装に負けるので
	// FORCE_COLOR=0 も明示する。
	envNoColor        = noColorEnvName + "=1"
	envForceColorZero = forceColorEnvName + "=0"

	// envMarker は wrap 配下であることを子 CLI へ知らせる目印。
	envMarker = "MANY_AI_CLI=1"
)

// childEnv は wrap した子 CLI へ渡す環境変数を、色方針（config.Hub.TerminalColor）に
// 従って組み立てる。方針が 3 択なのは、真偽値では利用者に嘘をつくことになるため。
// 「上書きをやめる」と「色を出さない」は別物で、起動元に NO_COLOR が無ければ
// 上書きをやめても色は出る。設定画面に「色なし」と書く以上、確実に消す口が要る。
//
//   - config.TerminalColorForce（既定）: FORCE_COLOR=3 と CLICOLOR_FORCE=1 を渡し、
//     継承した NO_COLOR は落とす。Hub を NO_COLOR=1 の環境（AI エージェントの
//     ハーネス配下など）から起こしても、ペインに色が出る。
//   - config.TerminalColorInherit: 起動元の環境をそのまま渡す。色が出るかは環境次第。
//   - config.TerminalColorOff: NO_COLOR=1 と FORCE_COLOR=0 を渡し、継承した
//     CLICOLOR_FORCE は落とす。色を出したくない人向け。
//
// 2026-09-01 の Linux 実機実測: FORCE_COLOR / CLICOLOR_FORCE だけを足した状態では
// claude の色指定が 0 → 91 個に戻る一方、codex は SGR 689 個すべて非色、grok も
// SGR 40 個すべて非色のままだった。NO_COLOR を落とすと codex 352 個・grok 11666 個の
// 色指定が出た。FORCE_COLOR を先に見る実装では上書きが効き、そうでない実装では
// NO_COLOR が勝っていた。詳細は docs/local/pending_wrap-inherits-no-color-from-hub-env.md。
//
// NO_COLOR の仕様では空文字は未設定と同じ扱いだが、「値が空か」ではなく「変数が
// 存在するか」だけを見る色判定もあるため、落とすときはキーごと落とす。名前の比較は
// 大文字小文字を無視する（Windows の環境変数名は case-insensitive）。
func childEnv(base []string, terminalColor string) []string {
	switch config.NormalizeTerminalColor(terminalColor) {
	case config.TerminalColorInherit:
		return appendBase(base)
	case config.TerminalColorOff:
		return append(appendBase(envWithout(base, clicolorForceEnvName)), envNoColor, envForceColorZero)
	default:
		return append(appendBase(envWithout(base, noColorEnvName)), envForceColor, envClicolorForce)
	}
}

// appendBase は色方針によらず常に渡すもの（端末の能力の訂正と wrap の目印）を足す。
func appendBase(env []string) []string {
	return append(append([]string(nil), env...), envTerm, envColorterm, envMarker)
}

// envWithout は指定した名前の環境変数を継承環境から取り除く。
func envWithout(env []string, names ...string) []string {
	out := make([]string, 0, len(env))
	for _, kv := range env {
		name, _, ok := strings.Cut(kv, "=")
		if ok && matchesEnvName(name, names) {
			continue
		}
		out = append(out, kv)
	}
	return out
}

func matchesEnvName(name string, names []string) bool {
	for _, want := range names {
		if strings.EqualFold(name, want) {
			return true
		}
	}
	return false
}
