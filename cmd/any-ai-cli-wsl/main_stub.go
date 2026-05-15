//go:build !windows

package main

import (
	"fmt"
	"os"
)

func main() {
	fmt.Fprintln(os.Stderr, "any-ai-cli-wsl is only available on Windows")
	os.Exit(1)
}
