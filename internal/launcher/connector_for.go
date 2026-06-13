package launcher

import "fmt"

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
