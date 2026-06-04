package launcher

import (
	"bufio"
	"io"
	"regexp"
)

// HubURLRe matches the Hub URL printed in the startup banner.
// Token is hex-only — anchoring to [0-9a-fA-F]+ avoids accidentally
// consuming trailing ANSI escapes or other non-whitespace junk that
// might appear on the same line in some terminals.
var HubURLRe = regexp.MustCompile(`http://127\.0\.0\.1:\d+/\?token=[0-9a-fA-F]+`)

// ScanForURL reads lines from r, writes each line to w with CRLF termination,
// and sends the first Hub URL found to urlCh (non-blocking). It returns when
// r reaches EOF or an error.
//
// The explicit "\r\n" emit ensures correct rendering even if the remote writes
// LF-only and the Windows console has not yet been put into PROCESSED_OUTPUT
// mode.
func ScanForURL(r io.Reader, w io.Writer, urlCh chan<- string) {
	s := bufio.NewScanner(r)
	s.Buffer(make([]byte, 0, 64*1024), 1024*1024)
	for s.Scan() {
		line := s.Text()
		_, _ = io.WriteString(w, line+"\r\n")
		if match := HubURLRe.FindString(line); match != "" {
			select {
			case urlCh <- match:
			default:
			}
		}
	}
}
