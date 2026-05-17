//go:build windows

package hub

var (
	procGetConsoleCP       = kernel32.MustFindProc("GetConsoleCP")
	procGetConsoleOutputCP = kernel32.MustFindProc("GetConsoleOutputCP")
)

func checkEncoding(parentShell string) encodingCheckResult {
	inCP, _, _ := procGetConsoleCP.Call()
	outCP, _, _ := procGetConsoleOutputCP.Call()
	inCPu := uint32(inCP)
	outCPu := uint32(outCP)
	return encodingCheckResult{
		IsWindows:      true,
		IsPowerShell:   isPowerShellShell(parentShell),
		InputCodepage:  inCPu,
		OutputCodepage: outCPu,
		IsUTF8:         inCPu == 65001 && outCPu == 65001,
	}
}
