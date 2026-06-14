package launcher

import (
	"fmt"
	"io"
)

// ConnectorFor returns the appropriate Connector for the given profile type.
// On non-Windows systems, ProfileTypeWSL returns an error.
func ConnectorFor(p Profile) (Connector, error) {
	switch p.Type {
	case ProfileTypeWSL:
		return connectorForWSL()
	case ProfileTypeSSH:
		return NewSSHConnector(), nil
	default:
		return nil, fmt.Errorf("unsupported profile type %q", p.Type)
	}
}

// quietable is implemented by connectors that can silence the child-process
// output they normally mirror to the console.
type quietable interface {
	setQuiet(io.Writer)
}

// ConnectorForQuiet returns a Connector like ConnectorFor but with its
// child-process output discarded. The Hub uses this to host SSH/WSL tunnels
// inside its own process: no console window is opened (see
// noWindowSysProcAttr) and the remote Hub URL + token is never echoed to the
// Hub's stdout.
func ConnectorForQuiet(p Profile) (Connector, error) {
	conn, err := ConnectorFor(p)
	if err != nil {
		return nil, err
	}
	if q, ok := conn.(quietable); ok {
		q.setQuiet(io.Discard)
	}
	return conn, nil
}
