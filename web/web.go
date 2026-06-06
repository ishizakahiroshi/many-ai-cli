// Package web は Hub UI の静的アセットを embed.FS で同梱する。
// バイナリ単独で動かせるようにし、Hub を任意の cwd で起動しても
// アセットが見つからない問題を防ぐ。
package web

import "embed"

// all: 接頭辞により、ドット/アンダースコア始まりのファイル（.gitkeep など）も
// 取りこぼさず embed する。これが無いと将来そうした名前のアセットが追加されても
// バイナリに含まれず、実行時に見つからない事故になる。
//
// web/dist は TypeScript フロントのビルド成果物。Go ビルド前に
// `cd web && bun install && bun run build` で生成しておくこと。
//
//go:embed all:dist
var FS embed.FS
