//go:build windows

package launcher

import (
	"fmt"
	"net"
	"strings"
	"time"
)

const (
	DefaultHubPort = 47777
	portStep       = 100
	maxPortProbes  = 10
	probeTimeout   = 200 * time.Millisecond
)

// PickPort returns the first port (DefaultHubPort, +portStep, +2*portStep, ...)
// that has no Windows-side listener responding within probeTimeout.
// This avoids the WSL-Hub-on-47777 case getting shadowed by a Windows-native
// Hub already bound to 47777 (Windows side wins for localhost forwarding).
// If all probes find something listening, fall back to DefaultHubPort and let
// the WSL-side serve port-scan as usual.
func PickPort() int {
	for i := 0; i < maxPortProbes; i++ {
		port := DefaultHubPort + i*portStep
		if !WindowsPortInUse(port) {
			return port
		}
	}
	return DefaultHubPort
}

// WindowsPortInUse reports whether a TCP listener is active on 127.0.0.1:port.
func WindowsPortInUse(port int) bool {
	conn, err := net.DialTimeout("tcp", fmt.Sprintf("127.0.0.1:%d", port), probeTimeout)
	if err != nil {
		return !isConnectionRefused(err)
	}
	_ = conn.Close()
	return true
}

func isConnectionRefused(err error) bool {
	if err == nil {
		return false
	}
	msg := strings.ToLower(err.Error())
	return strings.Contains(msg, "connection refused") ||
		strings.Contains(msg, "actively refused") ||
		strings.Contains(msg, "no connection could be made")
}
