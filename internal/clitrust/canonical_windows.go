//go:build windows

package clitrust

import (
	"strings"

	"golang.org/x/sys/windows"
)

// canonicalPath returns what Rust's std::fs::canonicalize followed by
// dunce::simplified returns for path — the spelling codex keys its projects
// table by (project_trust_key in codex-rs/config/src/loader/mod.rs,
// rust-v0.156.1): the final path of the opened file or directory, with every
// junction, symbolic link and 8.3 short name resolved and each component in
// its on-disk case, and the \\?\ prefix removed when that is safe.
//
// std::fs::canonicalize opens the path with no access rights and
// FILE_FLAG_BACKUP_SEMANTICS (so directories open too) and asks
// GetFinalPathNameByHandleW for FILE_NAME_NORMALIZED | VOLUME_NAME_DOS, which
// are both 0.
func canonicalPath(path string) (string, error) {
	name, err := windows.UTF16PtrFromString(path)
	if err != nil {
		return "", err
	}
	h, err := windows.CreateFile(name, 0,
		windows.FILE_SHARE_READ|windows.FILE_SHARE_WRITE|windows.FILE_SHARE_DELETE,
		nil, windows.OPEN_EXISTING, windows.FILE_FLAG_BACKUP_SEMANTICS, 0)
	if err != nil {
		return "", err
	}
	defer windows.CloseHandle(h)

	buf := make([]uint16, 512)
	for {
		n, err := windows.GetFinalPathNameByHandle(h, &buf[0], uint32(len(buf)), 0)
		if err != nil {
			return "", err
		}
		if int(n) < len(buf) {
			return simplifyVerbatimPath(windows.UTF16ToString(buf[:n])), nil
		}
		buf = make([]uint16, n+1)
	}
}

// simplifyVerbatimPath is dunce::simplified: \\?\C:\a\b becomes C:\a\b, but
// only when the plain form means the same thing. Any other verbatim form
// (\\?\UNC\server\share, a volume GUID) is kept as it is, as are paths longer
// than 260 bytes and paths with a component the plain form would reinterpret
// (a reserved device name such as CON, a trailing dot or space, a character
// Win32 forbids).
func simplifyVerbatimPath(p string) string {
	const prefix = `\\?\`
	if !strings.HasPrefix(p, prefix) {
		return p
	}
	rest := p[len(prefix):]
	if len(rest) < 3 || rest[1] != ':' || rest[2] != '\\' || !isASCIILetter(rest[0]) {
		return p
	}
	if len(p) > 260 {
		return p
	}
	for _, part := range strings.Split(rest[3:], `\`) {
		if part == "" {
			continue
		}
		if !isPlainWin32Name(part) || isReservedWin32Name(part) {
			return p
		}
	}
	return rest
}

func isASCIILetter(c byte) bool {
	return c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z'
}

// isPlainWin32Name is dunce's is_valid_filename.
func isPlainWin32Name(name string) bool {
	if name == "" || len(name) > 255 {
		return false
	}
	for i := 0; i < len(name); i++ {
		c := name[i]
		if c < 32 || strings.IndexByte(`<>:"/\|?*`, c) >= 0 {
			return false
		}
	}
	last := name[len(name)-1]
	return last != ' ' && last != '.'
}

// isReservedWin32Name is dunce's is_reserved: the part before the first dot,
// with trailing spaces trimmed, is a DOS device name ("con.txt" is reserved
// too).
func isReservedWin32Name(name string) bool {
	stem := name
	if i := strings.IndexByte(stem, '.'); i >= 0 {
		stem = stem[:i]
	}
	stem = strings.TrimRight(stem, " ")
	if len(stem) > 4 {
		return false
	}
	switch strings.ToUpper(stem) {
	case "AUX", "NUL", "PRN", "CON",
		"COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
		"LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9":
		return true
	}
	return false
}
