//go:build !windows

package main

import (
	"fmt"
	"os"
)

func main() {
	fmt.Fprintln(os.Stderr, "many-ai-cli-launcher is only available on Windows")
	os.Exit(1)
}
