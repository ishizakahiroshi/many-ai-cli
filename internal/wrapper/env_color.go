package wrapper

import "strings"

// envWithoutColorSuppressors drops NO_COLOR from an inherited environment.
//
// Hub のペインは xterm.js なので色を出せる。だから wrap は TERM / COLORTERM を
// 上書きし、FORCE_COLOR / CLICOLOR_FORCE も立てる。ところが Hub を NO_COLOR=1 の
// 環境（AI エージェントのハーネス配下など）から起こすと、それらを立てても色が
// 落ちる provider がある。
//
// 2026-09-01 の Linux 実機実測: FORCE_COLOR=3 / CLICOLOR_FORCE=1 を足した状態で、
// claude は色指定 91 個まで戻った一方、codex は SGR 689 個すべて非色、grok も
// SGR 40 個すべて非色だった。FORCE_COLOR を先に見る実装（Claude Code 同梱の bun
// ランタイム）では上書きが効き、そうでない実装では NO_COLOR が勝っていた。
//
// NO_COLOR の仕様では空文字は未設定と同じ扱いだが、「値が空かどうか」ではなく
// 「変数が存在するかどうか」だけを見る色判定もあるため、空文字を渡すのではなく
// キーごと落とす。詳細は docs/local/pending_wrap-inherits-no-color-from-hub-env.md。
//
// 名前の比較は大文字小文字を無視する（Windows の環境変数名は case-insensitive で、
// No_Color のような綴りで入ってくることがある）。
func envWithoutColorSuppressors(env []string) []string {
	out := make([]string, 0, len(env))
	for _, kv := range env {
		if name, _, ok := strings.Cut(kv, "="); ok && strings.EqualFold(name, "NO_COLOR") {
			continue
		}
		out = append(out, kv)
	}
	return out
}
