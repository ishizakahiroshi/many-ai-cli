// Package launcher provides connection profile management for the any-ai-cli launcher.
// Profiles are stored in ~/.any-ai-cli/launcher-profiles.yaml.
package launcher

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"gopkg.in/yaml.v3"
)

const (
	profilesFile    = "launcher-profiles.yaml"
	supportedVersion = 1
)

// ProfileType identifies the connection type of a profile.
type ProfileType string

const (
	ProfileTypeWSL ProfileType = "wsl"
	ProfileTypeSSH ProfileType = "ssh"
)

// SSHMode controls how the SSH profile connects to the remote Hub.
type SSHMode string

const (
	SSHModeServe  SSHMode = "serve"  // Start any-ai-cli serve on remote
	SSHModeTunnel SSHMode = "tunnel" // Forward port to an already-running Hub
)

// Profile is a single connection target stored in launcher-profiles.yaml.
// The json tags must mirror the yaml tags: the UI server exchanges profiles
// with the browser as JSON using the same snake_case field names, and
// encoding/json does not match `hub_port` to HubPort without an explicit tag
// (case folding ignores underscores, so the value would silently drop to 0).
type Profile struct {
	Name string      `yaml:"name" json:"name"`
	Type ProfileType `yaml:"type" json:"type"`

	// WSL-specific fields
	Distro string `yaml:"distro,omitempty" json:"distro,omitempty"` // empty = default WSL distro

	// SSH-specific fields
	Mode         SSHMode `yaml:"mode,omitempty" json:"mode,omitempty"`                   // serve (default) or tunnel
	Host         string  `yaml:"host,omitempty" json:"host,omitempty"`
	User         string  `yaml:"user,omitempty" json:"user,omitempty"`
	SSHPort      int     `yaml:"ssh_port,omitempty" json:"ssh_port,omitempty"`           // 0 = 22 or ssh config
	IdentityFile string  `yaml:"identity_file,omitempty" json:"identity_file,omitempty"` // empty = ssh default / agent

	// tunnel-mode specific
	TokenCommand string `yaml:"token_command,omitempty" json:"token_command,omitempty"` // required for tunnel

	// Common fields
	Binary  string `yaml:"binary,omitempty" json:"binary,omitempty"`     // CLI binary name on remote
	CWD     string `yaml:"cwd,omitempty" json:"cwd,omitempty"`           // working directory on remote
	HubPort int    `yaml:"hub_port,omitempty" json:"hub_port,omitempty"` // 0 = auto-select (not allowed for tunnel)
}

// ProfilesFile is the top-level structure of launcher-profiles.yaml.
type ProfilesFile struct {
	Version  int       `yaml:"version" json:"version"`
	LastUsed string    `yaml:"last_used,omitempty" json:"last_used,omitempty"`
	Profiles []Profile `yaml:"profiles,omitempty" json:"profiles,omitempty"`
}

// profilesPath returns the path to ~/.any-ai-cli/launcher-profiles.yaml.
func profilesPath() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("home dir: %w", err)
	}
	return filepath.Join(home, ".any-ai-cli", profilesFile), nil
}

// LoadProfiles reads launcher-profiles.yaml and returns the parsed ProfilesFile.
// If the file does not exist, an empty ProfilesFile with version=1 is returned.
// If version is unknown (> supportedVersion), an error is returned.
func LoadProfiles() (*ProfilesFile, error) {
	path, err := profilesPath()
	if err != nil {
		return nil, err
	}

	data, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		return &ProfilesFile{Version: supportedVersion}, nil
	}
	if err != nil {
		return nil, fmt.Errorf("read launcher profiles: %w", err)
	}

	// Handle empty file as fresh start.
	if len(strings.TrimSpace(string(data))) == 0 {
		return &ProfilesFile{Version: supportedVersion}, nil
	}

	var pf ProfilesFile
	if err := yaml.Unmarshal(data, &pf); err != nil {
		return nil, fmt.Errorf("parse launcher profiles: %w", err)
	}

	if pf.Version > supportedVersion {
		return nil, fmt.Errorf("unsupported launcher-profiles.yaml version %d (max supported: %d)", pf.Version, supportedVersion)
	}

	// Normalize host fields: "user@host" in Host takes precedence over User field.
	for i := range pf.Profiles {
		normalizeProfile(&pf.Profiles[i])
	}

	return &pf, nil
}

// SaveProfiles writes pf to ~/.any-ai-cli/launcher-profiles.yaml atomically
// (write to a temp file in the same directory, then rename).
func SaveProfiles(pf *ProfilesFile) error {
	path, err := profilesPath()
	if err != nil {
		return err
	}

	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return fmt.Errorf("mkdir launcher profiles dir: %w", err)
	}

	data, err := yaml.Marshal(pf)
	if err != nil {
		return fmt.Errorf("marshal launcher profiles: %w", err)
	}

	tmp, err := os.CreateTemp(dir, "launcher-profiles-*.yaml.tmp")
	if err != nil {
		return fmt.Errorf("create temp launcher profiles: %w", err)
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName) // no-op after successful Rename

	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return fmt.Errorf("write temp launcher profiles: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return fmt.Errorf("sync temp launcher profiles: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("close temp launcher profiles: %w", err)
	}
	if err := os.Chmod(tmpName, 0o600); err != nil {
		return fmt.Errorf("chmod temp launcher profiles: %w", err)
	}
	return os.Rename(tmpName, path)
}

// Validate checks all profiles in pf for correctness.
// Returns the first error found, or nil if all profiles are valid.
func Validate(pf *ProfilesFile) error {
	seen := make(map[string]bool)
	for i, p := range pf.Profiles {
		idx := i + 1 // 1-based for error messages

		// Name must be non-empty and unique.
		if p.Name == "" {
			return fmt.Errorf("profile[%d]: name is required", idx)
		}
		if seen[p.Name] {
			return fmt.Errorf("profile[%d]: duplicate name %q", idx, p.Name)
		}
		seen[p.Name] = true

		// Type must be wsl or ssh.
		switch p.Type {
		case ProfileTypeWSL:
			if err := validateWSL(p, idx); err != nil {
				return err
			}
		case ProfileTypeSSH:
			if err := validateSSH(p, idx); err != nil {
				return err
			}
		case "":
			return fmt.Errorf("profile[%d] %q: type is required", idx, p.Name)
		default:
			return fmt.Errorf("profile[%d] %q: unknown type %q (must be wsl or ssh)", idx, p.Name, p.Type)
		}
	}

	// Validate last_used refers to an existing profile (if set).
	// A missing reference is allowed (tolerated, not an error) per spec.
	_ = pf.LastUsed

	return nil
}

func validateWSL(p Profile, idx int) error {
	// mode is SSH-only; reject if specified on WSL profile.
	if p.Mode != "" {
		return fmt.Errorf("profile[%d] %q: mode is not applicable to wsl profiles", idx, p.Name)
	}
	// hub_port: 0 = auto-select is allowed for wsl.
	if err := validatePort(p.HubPort, false, "hub_port", p.Name, idx); err != nil {
		return err
	}
	return nil
}

func validateSSH(p Profile, idx int) error {
	// Resolve default mode.
	mode := p.Mode
	if mode == "" {
		mode = SSHModeServe
	}
	switch mode {
	case SSHModeServe:
		// hub_port 0 is allowed (auto-select).
		if err := validatePort(p.HubPort, false, "hub_port", p.Name, idx); err != nil {
			return err
		}
	case SSHModeTunnel:
		// hub_port must be 1–65535 for tunnel.
		if err := validatePort(p.HubPort, true, "hub_port", p.Name, idx); err != nil {
			return err
		}
		// token_command is required for tunnel.
		if strings.TrimSpace(p.TokenCommand) == "" {
			return fmt.Errorf("profile[%d] %q: token_command is required for tunnel mode", idx, p.Name)
		}
	default:
		return fmt.Errorf("profile[%d] %q: unknown mode %q (must be serve or tunnel)", idx, p.Name, p.Mode)
	}

	// ssh_port range check (0 = use ssh config / default 22).
	if err := validatePort(p.SSHPort, false, "ssh_port", p.Name, idx); err != nil {
		return err
	}

	return nil
}

// validatePort checks that port is within [1, 65535].
// If required is true, port=0 is also an error.
func validatePort(port int, required bool, field, name string, idx int) error {
	if port == 0 {
		if required {
			return fmt.Errorf("profile[%d] %q: %s must be 1–65535 (got 0)", idx, name, field)
		}
		return nil
	}
	if port < 1 || port > 65535 {
		return fmt.Errorf("profile[%d] %q: %s out of range (got %d, must be 1–65535)", idx, name, field, port)
	}
	return nil
}

// normalizeProfile rewrites "user@host" in Host into the separate User field.
// The Host part of "user@host" takes precedence over an existing User field.
func normalizeProfile(p *Profile) {
	if p.Type != ProfileTypeSSH {
		return
	}
	if !strings.Contains(p.Host, "@") {
		return
	}
	parts := strings.SplitN(p.Host, "@", 2)
	p.User = parts[0]
	p.Host = parts[1]
}
