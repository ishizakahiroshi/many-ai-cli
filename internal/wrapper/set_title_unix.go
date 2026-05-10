//go:build !windows

package wrapper

func setConsoleTitle(_ string) {}
func setConsoleIcon()           {}
