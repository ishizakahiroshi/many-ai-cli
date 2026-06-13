//go:build !windows

package launcher

import "errors"

// connectorForWSL always returns an error on non-Windows platforms because
// WSL is a Windows-only feature.
func connectorForWSL() (Connector, error) {
	return nil, errors.New("WSL profiles are only supported on Windows")
}
