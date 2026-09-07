package subscription

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/securefile"
)

// Seeding carries the user's own configuration into a subscription profile.
//
// Why this exists (measured 2026-08-23, recorded in
// docs/local/plan_subscription-profile-settings-carryover.md):
//
// Pointing a vendor CLI's directory switch at a fresh profile separates the
// login, which is the point. It also separates everything else that lives in
// that directory, which is not. A profile therefore used to start every session
// at the CLI's factory state: no user CLAUDE.md / AGENTS.md, no skills, no
// slash commands, no approval allowlist, no approval policy, no trusted
// folders. Nothing reported this — the session simply behaved as if the user
// had never configured the CLI, which is invisible from both sides and cost an
// afternoon before it was traced.
//
// Three rules keep this from turning into "many-ai-cli edits your CLI config":
//
//   - **Additive only.** An entry is carried in only when the profile does not
//     already have it. Nothing that exists in a profile is ever overwritten,
//     renamed, merged, or deleted, so a value the user changed inside a profile
//     always wins.
//   - **Inside our own tree only.** Every write lands under
//     ~/.many-ai-cli/subscriptions/. The user's real ~/.claude, ~/.codex and
//     ~/.grok are read and never written, so `uninstall` still removes
//     everything this creates.
//   - **Named entries only.** Each adapter lists the entries by name. There is
//     no "copy the whole directory", which would drag the credential across and
//     defeat the separation.
//
// Directories are linked rather than copied so that adding a skill later shows
// up in every profile without a re-seed; files are copied because the vendor
// CLI rewrites them and a link would write back into the default profile. Rule
// files (CLAUDE.md, AGENTS.md) are the one exception: the vendor CLI does not
// rewrite them, so when the user's own default copy is itself a symlink, the
// profile mirrors it as a symlink too instead of taking a snapshot.
type SeedKind int

const (
	// SeedCopyFile copies a file as-is. Used for files the vendor CLI rewrites
	// (settings.json, config.toml): a link would make the profile's edits land
	// in the user's default configuration.
	SeedCopyFile SeedKind = iota
	// SeedLinkDir points the profile at the default directory (a symlink, or a
	// junction on Windows where symlinks need a privilege). Used for content
	// the user maintains in one place: skills, slash commands, prompts.
	SeedLinkDir
	// SeedJSONKeys writes a new JSON file holding only the named top-level keys
	// of the source. Used for Claude Code's .claude.json, which mixes account
	// identity with a few genuine preferences; copying the file whole would
	// carry the default account's identity into the profile.
	SeedJSONKeys
	// SeedMirrorFile mirrors a rule file: if the user's default copy is itself a
	// symlink, the profile gets a symlink to the same resolved target so both
	// always read one file; otherwise it is copied like SeedCopyFile. Used only
	// for rule files (CLAUDE.md, AGENTS.md) that the vendor CLI does not
	// rewrite — never for settings.json/config.toml, where a link would let a
	// profile's login write back into the user's default configuration.
	SeedMirrorFile
)

// SeedEntry is one thing carried from the user's default configuration into a
// profile directory.
type SeedEntry struct {
	// Source is the absolute path in the user's default configuration.
	Source string
	// Dest is the path relative to the profile directory.
	Dest string
	Kind SeedKind
	// Keys limits SeedJSONKeys to these top-level JSON keys.
	Keys []string
	// Label is the human name shown by `many-ai-cli doctor`.
	Label string
}

// ProfileSeeder is the optional half of Adapter. A provider that loses nothing
// when its directory is switched simply does not implement it — OpenCode is the
// real case, because only XDG_DATA_HOME moves and its config stays shared.
type ProfileSeeder interface {
	// SeedEntries resolves the entries against the user's actual default
	// configuration location. An empty result is a normal state (no default
	// configuration found), not an error.
	SeedEntries() []SeedEntry
}

// SeedResult reports what one seeding pass did. It is informational: seeding
// failures never block a session from starting, because a session with a
// factory-state configuration is still a working session.
type SeedResult struct {
	// Applied holds the Dest of every entry carried in during this pass.
	Applied []string
	// Failed holds the Dest of every entry that could not be carried in.
	Failed []string
	// Degraded holds the Dest of every SeedMirrorFile entry that wanted to be a
	// symlink but had to fall back to a copy (e.g. Windows without Developer
	// Mode). The entry still counts as Applied; this is only for surfacing why
	// a profile's rule file will not track later edits to the default.
	Degraded []string
}

// Any reports whether the pass did anything worth logging.
func (r SeedResult) Any() bool {
	return len(r.Applied) > 0 || len(r.Failed) > 0 || len(r.Degraded) > 0
}

// errNothingToSeed means the source existed but held nothing worth writing.
// Treated as "skip", not as a failure.
var errNothingToSeed = errors.New("nothing to seed")

// seedEntriesFor returns the entries the provider's adapter declares.
func seedEntriesFor(provider string) []SeedEntry {
	adapter, ok := AdapterFor(provider)
	if !ok {
		return nil
	}
	seeder, ok := adapter.(ProfileSeeder)
	if !ok {
		return nil
	}
	return seeder.SeedEntries()
}

// PendingSeedEntries returns the entries whose source exists and whose
// destination does not, i.e. exactly what a seeding pass would carry in.
//
// `many-ai-cli doctor` calls this to report drift without changing anything:
// the seed only runs when the Hub prepares a profile, so a profile created
// before this existed, or one where the user later added a skill directory to
// their default configuration, stays visible instead of silently lagging.
func PendingSeedEntries(provider, profileDir string) []SeedEntry {
	if strings.TrimSpace(profileDir) == "" {
		return nil
	}
	var out []SeedEntry
	for _, entry := range seedEntriesFor(provider) {
		if entry.Source == "" || entry.Dest == "" {
			continue
		}
		if _, err := os.Lstat(entry.Source); err != nil {
			continue
		}
		// Lstat, not Stat: a dangling link still counts as "the profile has
		// something here", and replacing it is the user's call, not ours.
		if _, err := os.Lstat(filepath.Join(profileDir, entry.Dest)); err == nil {
			continue
		}
		out = append(out, entry)
	}
	return out
}

// SeedProfileDir carries every pending entry into profileDir.
//
// It never returns an error. Each entry is independent, and a failure on one
// (an unreadable source, a filesystem that refuses links) must not stop the
// others or the session that asked for the profile.
func SeedProfileDir(provider, profileDir string) SeedResult {
	var result SeedResult
	for _, entry := range PendingSeedEntries(provider, profileDir) {
		dest := filepath.Join(profileDir, entry.Dest)
		var err error
		var degraded bool
		switch entry.Kind {
		case SeedCopyFile:
			err = copySeedFile(entry.Source, dest)
		case SeedLinkDir:
			err = linkSeedDir(entry.Source, dest)
		case SeedJSONKeys:
			err = writeSeedJSONKeys(entry.Source, dest, entry.Keys)
		case SeedMirrorFile:
			degraded, err = mirrorSeedFile(entry.Source, dest)
		default:
			continue
		}
		switch {
		case errors.Is(err, errNothingToSeed):
			// Source held none of the named keys. Nothing to report.
		case err != nil:
			result.Failed = append(result.Failed, entry.Dest)
		default:
			result.Applied = append(result.Applied, entry.Dest)
			if degraded {
				result.Degraded = append(result.Degraded, entry.Dest)
			}
		}
	}
	return result
}

// copySeedFile copies a regular file. Directories are refused rather than
// walked: a recursive copy is how "carry the settings" turns into "carry the
// credential" the first time a vendor CLI moves a file.
func copySeedFile(src, dst string) error {
	info, err := os.Stat(src)
	if err != nil {
		return err
	}
	if info.IsDir() {
		return errors.New("seed source is a directory")
	}
	data, err := os.ReadFile(src) // #nosec G304 -- src comes from an adapter's fixed entry list
	if err != nil {
		return err
	}
	// The profile directory already carries a private DACL with inheritance
	// (EnsureProfileDir), so a file created inside it is restricted on arrival.
	return securefile.WriteAtomic(dst, data, 0o600)
}

// linkSeedDir points dst at src. The link keeps one copy of the user's skills
// and commands, so adding one later reaches every profile with no re-seed.
func linkSeedDir(src, dst string) error {
	info, err := os.Stat(src)
	if err != nil {
		return err
	}
	if !info.IsDir() {
		return errors.New("seed source is not a directory")
	}
	if err := os.MkdirAll(filepath.Dir(dst), config.DirMode); err != nil {
		return err
	}
	return linkDir(src, dst)
}

// IsLinkedRuleFile reports whether src is a symlink. Exported so `many-ai-cli
// doctor` can tell a profile's plain copy of a rule file apart from a case
// where there is nothing to compare (the default itself is a symlink and the
// profile correctly mirrors it as one).
func IsLinkedRuleFile(src string) bool {
	info, err := os.Lstat(src)
	if err != nil {
		return false
	}
	return info.Mode()&os.ModeSymlink != 0
}

// symlinkFile is a package variable so tests can force the fallback path
// without needing an environment that denies real symlinks.
var symlinkFile = os.Symlink

// mirrorSeedFile carries a rule file into dst. If src is a symlink it makes
// dst a symlink to the same resolved absolute target, so the default and
// every profile read one physical file with no re-seed needed after an edit.
// If src is a regular file, or the symlink cannot be created (Windows without
// Developer Mode: junctions do not apply to files), it falls back to a plain
// copy and reports that as degraded.
func mirrorSeedFile(src, dst string) (degraded bool, err error) {
	if !IsLinkedRuleFile(src) {
		return false, copySeedFile(src, dst)
	}
	target, err := filepath.EvalSymlinks(src)
	if err != nil {
		// A dangling or unreadable link: fall back to the copy path, which will
		// itself fail informatively via os.Stat/os.ReadFile.
		return true, copySeedFile(src, dst)
	}
	if err := os.MkdirAll(filepath.Dir(dst), config.DirMode); err != nil {
		return false, err
	}
	if err := symlinkFile(target, dst); err != nil {
		return true, copySeedFile(src, dst)
	}
	return false, nil
}

// writeSeedJSONKeys writes dst holding only the named top-level keys of src.
//
// The values are copied as raw JSON without being interpreted. many-ai-cli does
// not know or care what `claudeInChromeDefaultEnabled` means; it only knows the
// user set it and that a new profile should not silently disagree.
func writeSeedJSONKeys(src, dst string, keys []string) error {
	if len(keys) == 0 {
		return errNothingToSeed
	}
	data, err := os.ReadFile(src) // #nosec G304 -- src comes from an adapter's fixed entry list
	if err != nil {
		return err
	}
	var all map[string]json.RawMessage
	if err := json.Unmarshal(data, &all); err != nil {
		// A vendor file we cannot parse is not an error worth surfacing: the
		// format is theirs and it changes with their releases.
		return errNothingToSeed
	}
	picked := make(map[string]json.RawMessage, len(keys))
	for _, key := range keys {
		if value, ok := all[key]; ok {
			picked[key] = value
		}
	}
	if len(picked) == 0 {
		return errNothingToSeed
	}
	encoded, err := json.MarshalIndent(picked, "", "  ")
	if err != nil {
		return err
	}
	return securefile.WriteAtomic(dst, append(encoded, '\n'), 0o600)
}

// defaultHomeFromEnv reads the vendor CLI's own directory switch so that a user
// who already moved their default configuration is seeded from where it
// actually is.
//
// A value pointing inside many-ai-cli's own subscriptions tree is ignored. That
// happens when the Hub was started from a session that many-ai-cli itself
// wrapped, and seeding a new profile from another profile would carry the wrong
// account's state into it.
func defaultHomeFromEnv(envVar string) string {
	value := strings.TrimSpace(os.Getenv(envVar))
	if value == "" {
		return ""
	}
	if insideSubscriptionsTree(value) {
		return ""
	}
	return value
}

func insideSubscriptionsTree(path string) bool {
	dir, err := config.Dir()
	if err != nil {
		return false
	}
	root := config.SubscriptionsRoot(dir)
	rel, err := filepath.Rel(root, filepath.Clean(path))
	if err != nil {
		return false
	}
	return rel != ".." && !strings.HasPrefix(rel, ".."+string(filepath.Separator))
}

// vendorDefaultDir returns the directory the vendor CLI uses when no profile is
// selected: its own directory switch when the user set one, otherwise the
// documented default under the home directory.
func vendorDefaultDir(envVar, homeRelative string) string {
	if dir := defaultHomeFromEnv(envVar); dir != "" {
		return dir
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return ""
	}
	return filepath.Join(home, homeRelative)
}
