//go:build !windows

package hub

func checkEncoding(parentShell string) encodingCheckResult {
	return encodingCheckResult{
		IsWindows:    false,
		IsPowerShell: isPowerShellShell(parentShell),
		IsUTF8:       true,
	}
}
