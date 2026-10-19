//go:build !windows

package hub

import (
	"fmt"
	"os"
)

// setConsoleTitle emits the xterm OSC 0 escape sequence so that terminal
// emulators (xterm, GNOME Terminal, Terminal.app, iTerm2, kitty, alacritty,
// wezterm, etc.) update the window/tab title. Terminals that don't understand
// the sequence simply ignore it.
func setConsoleTitle(title string) {
	fmt.Fprintf(os.Stdout, "\x1b]0;%s\x07", title)
}

func setConsoleIcon() {}
