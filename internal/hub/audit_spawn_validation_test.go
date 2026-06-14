package hub

import (
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

// TestSpawnValidModelLabel は spawn の model / label / label_prefix 許可リスト検証を確認する。
// 実在するモデル名・ラベルは通り、空白やシェル/バッチメタ文字を含む値は拒否される
// （Windows の cmd.exe /c シム経路への引数注入の多層防御）。
func TestSpawnValidModelLabel(t *testing.T) {
	valid := []string{
		"",                            // 空は未指定扱いで許可
		"claude-opus-4-20250514",      // 典型的な Claude モデル名
		"gpt-5",                       // 典型的な OpenAI モデル名
		"anthropic/claude-3.5-sonnet", // provider/model 形式
		"qwen2.5-coder:14b",           // Ollama 形式（: と . を含む）
		"model@v1.2_beta+1",           // @ _ + . を含むラベル
		"my-grid",                     // label_prefix
	}
	for _, v := range valid {
		if !spawnValidModelLabel(v) {
			t.Errorf("spawnValidModelLabel(%q) = false, want true", v)
		}
	}

	invalid := []string{
		"has space",  // 空白
		`x"&calc&"y`, // CVE-2024-24576 系の cmd メタ文字
		"a|b",        // パイプ
		"a&b",        // アンパサンド
		"a>b",        // リダイレクト
		"a<b",        // リダイレクト
		"a^b",        // cmd エスケープ
		"a%PATH%b",   // cmd 変数展開
		"a(b)",       // 括弧
		"a;b",        // セミコロン
		"a\tb",       // タブ
		"a\nb",       // 改行
		"a`b",        // バッククォート
		"a$b",        // ドル
	}
	for _, v := range invalid {
		if spawnValidModelLabel(v) {
			t.Errorf("spawnValidModelLabel(%q) = true, want false", v)
		}
	}
}

// TestSpawnCwdTooBroad は spawn の cwd 広域ルート拒否（監査 #1 折衷案）を確認する。
// ドライブ/FS ルート・ホーム・全ユーザー親・主要システムルートは「そのルート自身」のみ拒否され、
// 配下の通常プロジェクトフォルダは許可される（git の有無に依存しない）。
func TestSpawnCwdTooBroad(t *testing.T) {
	var broad, okPaths []string
	if runtime.GOOS == "windows" {
		broad = []string{`C:\`, `D:\`, `C:\Windows`, `C:\Program Files`, `C:\Users`}
		okPaths = []string{`C:\dev\myproject`, `C:\Users\someone\proj`, `C:\Windows\Temp\sub`}
	} else {
		broad = []string{"/", "/etc", "/usr", "/var", "/home", "/root"}
		okPaths = []string{"/home/someone/proj", "/etc/myapp/conf", "/var/www/site", "/opt/app"}
	}
	// ホーム自身と全ユーザー親は両 OS 共通で拒否対象。
	if home, err := os.UserHomeDir(); err == nil && home != "" {
		broad = append(broad, home, filepath.Dir(home))
	}
	for _, c := range broad {
		if !spawnCwdTooBroad(c) {
			t.Errorf("spawnCwdTooBroad(%q) = false, want true (broad root)", c)
		}
	}
	for _, c := range okPaths {
		if spawnCwdTooBroad(c) {
			t.Errorf("spawnCwdTooBroad(%q) = true, want false (normal project folder)", c)
		}
	}
}
