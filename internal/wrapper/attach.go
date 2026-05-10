package wrapper

import (
	"fmt"
	"io"

	"ai-cli-hub/internal/proto"
)

// HandleAttach は attach_file メッセージの Inject 文字列を ptySink へ書き込む。
func HandleAttach(msg proto.Message, ptySink io.Writer) error {
	if msg.Inject == "" {
		return nil
	}
	if _, err := ptySink.Write([]byte(msg.Inject)); err != nil {
		return fmt.Errorf("inject write: %w", err)
	}
	return nil
}
