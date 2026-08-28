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
//
// 認証情報が設定値と同居する既知の形式もここで拒否する。ファイル名に secret や
// credential を含まないものほど警戒から外れる（.npmrc は「設定ファイル」の顔をしている）。
var secretReadDeniedBasenames = map[string]bool{
	"any-ai-cli.db":     true,
	"any-ai-cli.db-wal": true,
	"any-ai-cli.db-shm": true,
	// SSH の周辺ファイル。秘密鍵そのものは isSSHPrivateKeyName が見る。
	"authorized_keys": true,
	"known_hosts":     true,
	// 設定値と認証情報が同居する形式。
	".netrc":           true,
	"_netrc":           true, // Windows の curl / git が使う綴り
	".npmrc":           true,
	".pypirc":          true,
	".git-credentials": true,
	".htpasswd":        true,
}

// isSSHPrivateKeyName は OpenSSH の既定鍵名かを返す（小文字の basename を渡す）。
//
// 2026-08-17 の監査 F-61 相当の指摘: 以前は id_rsa の前方一致だけを見ていたため、
// 今どき既定になっている id_ed25519 も、id_ecdsa も id_dsa も素通りしていた。
// 「危ないものを 1 つずつ思い出して並べる」形は、思い出さなかったものが
// 例外も警告も無く通る。鍵名は ssh-keygen の既定が有限なので列挙で足りるが、
// 追加するときは .pub / -cert.pub のような派生も一緒に通ることを意識する
// （前方一致にしているのはそのため）。
func isSSHPrivateKeyName(base string) bool {
	for _, stem := range [...]string{"id_rsa", "id_dsa", "id_ecdsa", "id_ecdsa_sk", "id_ed25519", "id_ed25519_sk"} {
		if strings.HasPrefix(base, stem) {
			return true
		}
	}
	return false
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
//   - 鍵ファイル: *.pem / *.key / OpenSSH の既定鍵名（isSSHPrivateKeyName）
//   - 資格情報: ファイル名に credentials を含むもの、および認証情報が設定値と
//     同居する既知の形式（.netrc / .npmrc / .pypirc / .git-credentials ほか）
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
	if isSSHPrivateKeyName(base) {
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
