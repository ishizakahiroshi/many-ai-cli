//go:build !windows

package wrapper

import (
	"fmt"
	"os"
)

// setConsoleTitle emits the xterm OSC 0 escape sequence (see the matching
// implementation in internal/hub/set_title_unix.go for details).
func setConsoleTitle(title string) {
	fmt.Fprintf(os.Stdout, "\x1b]0;%s\x07", title)
}

func setConsoleIcon() {}
