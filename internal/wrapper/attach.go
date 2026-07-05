package wrapper

import (
	"fmt"
	"io"

	"many-ai-cli/internal/proto"
)

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
