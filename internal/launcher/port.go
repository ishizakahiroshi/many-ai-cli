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
// that has no listener responding within probeTimeout.
// If all probes find something listening, fall back to DefaultHubPort.
func PickPort() int {
	for i := 0; i < maxPortProbes; i++ {
		port := DefaultHubPort + i*portStep
		if !PortInUse(port) {
			return port
		}
	}
	return DefaultHubPort
}

// PortInUse reports whether a TCP listener is active on 127.0.0.1:port.
func PortInUse(port int) bool {
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
