package hub

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

// lookPathLikeSpawn は exec.LookPath と同じ結果を、spawn が子へ渡すのと同じ
// PATH（sanitizeEnv と同じ expandPathEntries で作り直したもの）に対して返す。
//
// Hub プロセスの PATH は親プロセスの起動時点のまま固まっている。Windows で
// Hub（またはその親）の起動後に CLI を入れた・入れ直した場合、インストーラーが
// 書き換えたレジストリの PATH は spawn 側にだけ見え、exec.LookPath を直接使う
// 導入状況・バージョン確認・更新の判定やモデル一覧の取得には見えず「起動できるのに
// 未インストール」になる。CLI の場所を探す処理はこの関数（providerCommandLookPath）を通すこと。
//
// macOS / Linux では expandPathEntries が何もしないので exec.LookPath と同じ。
func lookPathLikeSpawn(file string) (string, error) {
	if strings.ContainsRune(file, '/') || strings.ContainsRune(file, filepath.Separator) || filepath.VolumeName(file) != "" {
		return exec.LookPath(file)
	}
	for _, dir := range expandPathEntries(filepath.SplitList(os.Getenv("PATH"))) {
		// 相対エントリは exec.LookPath でも ErrDot で失敗扱いになるので飛ばす。
		if strings.TrimSpace(dir) == "" || !filepath.IsAbs(dir) {
			continue
		}
		// パス区切りを含む名前を渡すと、exec.LookPath はそのファイルだけを
		// 調べる（Windows では PATHEXT も当てる）。
		if path, err := exec.LookPath(filepath.Join(dir, file)); err == nil {
			return path, nil
		}
	}
	return "", &exec.Error{Name: file, Err: exec.ErrNotFound}
}

// sanitizeEnv は子プロセスへ渡す環境変数列の PATH (Windows では "Path" / "PATH" の
// 大小無視) を整える:
//   - 連続セミコロンによる空エントリを除去する。
//   - Windows のみ `%VAR%` 形式の未展開エントリを spawn 直前に再展開する
//     (`expandPathEntries`)。
//
// Windows ではユーザー Path に `;;` のような空エントリが混ざっていると、MSIX/UWP
// アプリ (例: WindowsApps 経由の OneCommander 等) から起動された子プロセスへ env を
// 継承する過程で **最初の空エントリ以降が打ち切られる** ケースがある。これが起きると
// 後段に並ぶ `.local\bin` 等のディレクトリが見えなくなり、wrap プロセス内の
// `exec.LookPath("claude")` が失敗 → セッションが即 disconnect する。
//
// 同様に pnpm setup が永続 USER PATH へ `%PNPM_HOME%\bin` を書き込む方式の場合、
// Hub プロセス起動時に `PNPM_HOME` が未 export だと REG_EXPAND_SZ が展開できず
// pnpm bin のエントリが脱落する。spawn 直前に再展開することで救済する。
//
// many-ai-cli 自身は spawn 直前に env を sanitize することで、永続 Path のゴミを
// ユーザーが気づかなくても claude / codex が見える状態を保証する。
func sanitizeEnv(env []string) []string {
	out := make([]string, 0, len(env))
	for _, kv := range env {
		eq := strings.IndexByte(kv, '=')
		if eq <= 0 {
			out = append(out, kv)
			continue
		}
		key := kv[:eq]
		if !strings.EqualFold(key, "Path") {
			out = append(out, kv)
			continue
		}
		raw := kv[eq+1:]
		parts := strings.Split(raw, string(os.PathListSeparator))
		parts = expandPathEntries(parts)
		cleaned := make([]string, 0, len(parts))
		for _, p := range parts {
			if strings.TrimSpace(p) == "" {
				continue
			}
			cleaned = append(cleaned, p)
		}
		out = append(out, key+"="+strings.Join(cleaned, string(os.PathListSeparator)))
	}
	return out
}
