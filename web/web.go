// Package web は Hub UI の静的アセットを embed.FS で同梱する。
// バイナリ単独で動かせるようにし、Hub を任意の cwd で起動しても
// アセットが見つからない問題を防ぐ。
package web

import "embed"

//go:embed src
var FS embed.FS
