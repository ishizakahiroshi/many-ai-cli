package wrapper

import (
	"fmt"
	"io"

	"many-ai-cli/internal/proto"
)

// looksLikeInjectPath は data が「@」直後の絶対パス形状かを判定する。
// PTYW-7 (report_bug_security_quality_audit_2026-07-05.md) の分岐条件で使う。
// - POSIX 絶対パス: `/` 始まり
// - Windows ドライブレター: `[A-Za-z]:[\\/]`
// これ以外 (相対パス風・@mention 風・任意テキスト) は false を返し、
// wrapper.go 側で旧経路 (150ms 遅延分割) に入らせず通常書き込みに倒す。
func looksLikeInjectPath(data []byte) bool {
	if len(data) == 0 {
		return false
	}
	if data[0] == '/' {
		return true
	}
	if len(data) >= 3 {
		c := data[0]
		if ((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z')) && data[1] == ':' && (data[2] == '\\' || data[2] == '/') {
			return true
		}
	}
	return false
}

// HandleAttach は attach_file メッセージの Inject 文字列を ptySink へ書き込む。
// PTYW-6 (report_bug_security_quality_audit_2026-07-05.md): 単一 Write では
// partial write (Windows go-pty の Pty.Write が短い書き込みで返す可能性) に
// 対応できないため、writePTY 系と同じく残りバイトを追い書きする defense-in-depth
// ループにする。Unix の *os.File.Write は Go 標準ライブラリで内部ループ済みだが、
// 抽象化された io.Writer 契約は「短い書き込みでも err なし」を許容する。
func HandleAttach(msg proto.Message, ptySink io.Writer) error {
	if msg.Inject == "" {
		return nil
	}
	data := []byte(msg.Inject)
	for len(data) > 0 {
		n, err := ptySink.Write(data)
		if err != nil {
			return fmt.Errorf("inject write: %w", err)
		}
		if n <= 0 {
			return fmt.Errorf("inject write: %w", io.ErrShortWrite)
		}
		data = data[n:]
	}
	return nil
}
