package hub

import (
	"net/http"
	"os"
	"path/filepath"
	"strings"
)

// filesScopeRestricted は「読み取り系 Files API で許可ルート制限を掛けるか」を返す。
//
// many-ai-cli は Hub ホスト上の単一ユーザー向けツールであり、直 loopback のブラウザは
// OS ユーザー本人と同一視できる。本人はエクスプローラで任意のファイルを開けるうえ、
// AI CLI 本体（Claude Code / Codex 等）も同じ OS 権限で任意のファイルを読めるため、
// Hub の読み取りだけを cwd / git root に閉じ込めても境界としては機能しない
// （POST /api/spawn は provider="shell" を受け付け、spawnCwdTooBroad はドライブ直下や
// ホーム自身しか拒否しないので、token を持つ呼び出し元には上位互換の経路が開いている）。
// そのため直 loopback からの読み取りは許可ルートで制限しない。
//
// 一方 tailscale serve / trusted_networks / スマホ等の「論理的にリモート」な呼び出しは
// 操作者＝OS ユーザー本人とは限らないため、従来どおり
// cwd / git root / attachments / orchestration + チャット言及フォールバックに閉じる。
// この直 loopback / 論理リモートの二分は POST /api/list-subdirs
// （misc_handlers.go の listSubdirsAllowedRemote）が既に採っている形と同じ。
//
// 書き込み系（files-save / create / mkdir / move / rename / delete）は本関数を使わない。
// リポ外を誤って壊さないための実利があるため cwd / git root のまま据え置く。
func (s *Server) filesScopeRestricted(r *http.Request) bool {
	return s.isLogicallyRemote(r)
}

// secretReadDeniedExtensions は許可ルート外の読み取りで拒否する拡張子（小文字）。
var secretReadDeniedExtensions = map[string]bool{
	".pem": true,
	".key": true,
}

// secretReadDeniedBasenames は許可ルート外の読み取りで拒否するファイル名（小文字・完全一致）。
//
// any-ai-cli.db は Hub のセッション履歴 SQLite（プロジェクト改名前の名前のまま運用中。
// 組み立ては internal/sessionstore/store.go の OpenForLogDir）。全セッションのチャット本文が
// 入るため、hub.log_dir を既定の ~/.many-ai-cli/logs から移設した構成でも拾えるよう、
// ディレクトリではなく名前で拒否する。-wal / -shm は SQLite の副ファイルで、
// 未チェックポイントの本文がそのまま残る。
var secretReadDeniedBasenames = map[string]bool{
	"any-ai-cli.db":     true,
	"any-ai-cli.db-wal": true,
	"any-ai-cli.db-shm": true,
}

// manyAiCliHomeDir は ~/.many-ai-cli を返す。
func manyAiCliHomeDir() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(home, ".many-ai-cli"), nil
}

// isSecretReadDenied は「許可ルート外のファイル内容をブラウザへ返す経路」で拒否すべき
// 秘密情報ファイルかを返す。対象は次の 5 種類:
//
//   - 鍵ファイル: *.pem / *.key / id_rsa*
//   - 資格情報: ファイル名に credentials を含むもの
//   - 環境変数ファイル: .env / .env.local / .env.<環境名>
//   - Hub のセッション履歴 DB: any-ai-cli.db（+ -wal / -shm）
//   - Hub 設定: ~/.many-ai-cli/config.yaml* （config.yaml とその複製）
//
// config.yaml は Hub token を平文 YAML で持つ（internal/config/config.go の Config.Token）。
// 完全一致ではなく前方一致にしているのは、設定を書き換える前に同じディレクトリへ
// config.yaml.bak のような複製を残す運用があり、完全一致だと複製側から同じ token が
// 素通しになるため。
//
// 適用範囲を許可ルート外に限るのは意図的。本 denylist の目的は「リポ外の読み取りを
// 開放したことで新たに露出する範囲を絞る」ことだけで、プロジェクト配下のファイルの
// 扱いは従来どおりに保つ（リポ内の .env や鍵ファイルが急に読めなくなる退行を避ける）。
//
// 「既定のアプリで開く」「フォルダを開く」には適用しない。これらは Hub ホスト上で
// 開くだけでブラウザへ中身を送らないため、エクスプローラで開くのと露出が変わらない。
func isSecretReadDenied(absPath string) bool {
	if absPath == "" {
		return false
	}
	if secretReadDeniedExtensions[strings.ToLower(filepath.Ext(absPath))] {
		return true
	}
	base := strings.ToLower(filepath.Base(absPath))
	if secretReadDeniedBasenames[base] {
		return true
	}
	if strings.HasPrefix(base, "id_rsa") {
		return true
	}
	if strings.Contains(base, "credentials") {
		return true
	}
	// .env / .env.local / .env.production など。direnv の .envrc は対象外
	// （設定スクリプトであり、"." 区切りの環境別ファイルとは別物のため）。
	if base == ".env" || strings.HasPrefix(base, ".env.") {
		return true
	}
	if strings.HasPrefix(base, "config.yaml") {
		if dir, err := manyAiCliHomeDir(); err == nil && isUnder(absPath, dir) {
			return true
		}
	}
	return false
}
