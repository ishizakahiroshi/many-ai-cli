package wrapper

import "strings"

const (
	// noColorEnvName は「色を出すな」という利用者の意思表示（https://no-color.org）。
	// 値ではなく存在で判定する実装があるため、落とすときはキーごと落とす。
	noColorEnvName = "NO_COLOR"

	// envTerm / envColorterm は端末の能力の訂正。Hub のペインは xterm.js なので、
	// 起動元の端末が dumb でも 256 色・truecolor を出せる。
	envTerm      = "TERM=xterm-256color"
	envColorterm = "COLORTERM=truecolor"

	// envForceColor の 3 は色深度のレベル指定で 24bit（truecolor）を意味する。
	// 1=16 色 / 2=256 色 / 3=24bit で、Node・bun・chalk 系が共通で解釈する。
	// xterm.js が truecolor を出せるので最大の 3 を指定する。
	envForceColor = "FORCE_COLOR=3"
	// envClicolorForce は Rust 系 CLI（anstream / anstyle-query 等）が見る強制フラグ。
	// こちらはレベルではなく on/off で、1 が on。
	envClicolorForce = "CLICOLOR_FORCE=1"

	// envMarker は wrap 配下であることを子 CLI へ知らせる目印（承認 UI の出し分け等）。
	envMarker = "MANY_AI_CLI=1"
)

// childEnv は wrap した子 CLI へ渡す環境変数を組み立てる。
//
// TERM / COLORTERM は常に上書きする。Hub のペインは xterm.js なので、起動元の端末が
// 何であれ 256 色・truecolor を出せる。これは「端末の能力の訂正」であって利用者の
// 好みの上書きではない。
//
// forceColor が true（既定）のときは、さらに次を行う。
//
//   - FORCE_COLOR=3（24bit）と CLICOLOR_FORCE=1（Rust 系 CLI 向け）を立てる
//   - 継承した NO_COLOR を落とす
//
// NO_COLOR を落とすのは、Hub を NO_COLOR=1 の環境（AI エージェントのハーネス配下など）
// から起こすと、TERM を上書きしても子 CLI が色を落とすため。2026-09-01 の Linux 実機実測:
// FORCE_COLOR / CLICOLOR_FORCE だけを足した状態では claude の色指定が 0 → 91 個に戻る
// 一方、codex は SGR 689 個すべて非色、grok も SGR 40 個すべて非色のままだった。
// NO_COLOR を落とすと codex 352 個・grok 11666 個の色指定が出た。
// FORCE_COLOR を先に見る実装では上書きが効き、そうでない実装では NO_COLOR が勝っていた。
//
// NO_COLOR の仕様では空文字は未設定と同じ扱いだが、「値が空か」ではなく「変数が存在するか」
// だけを見る色判定もあるため、空文字を渡すのではなくキーごと落とす。名前の比較は大文字小文字を
// 無視する（Windows の環境変数名は case-insensitive で、No_Color のような綴りで入ってくる）。
//
// **forceColor が false のときは NO_COLOR をそのまま渡し、FORCE_COLOR / CLICOLOR_FORCE も
// 付けない。** NO_COLOR は端末の能力ではなく利用者の意思表示なので、黙って無視し続ける形には
// しない。config.yaml の hub.force_color: false で 1 行で降りられる
// （docs/local/pending_wrap-inherits-no-color-from-hub-env.md の決定）。
func childEnv(base []string, forceColor bool) []string {
	env := append(append([]string(nil), base...), envTerm, envColorterm, envMarker)
	if !forceColor {
		return env
	}
	return append(envWithoutColorSuppressors(env), envForceColor, envClicolorForce)
}

// envWithoutColorSuppressors は継承環境から NO_COLOR を取り除く。理由は childEnv 参照。
func envWithoutColorSuppressors(env []string) []string {
	out := make([]string, 0, len(env))
	for _, kv := range env {
		if name, _, ok := strings.Cut(kv, "="); ok && strings.EqualFold(name, noColorEnvName) {
			continue
		}
		out = append(out, kv)
	}
	return out
}
