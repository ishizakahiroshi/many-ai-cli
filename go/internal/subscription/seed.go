package subscription

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"sort"
	"strings"

	"github.com/pelletier/go-toml/v2"

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
//   - **Additive only, with one named exception.** An entry is carried in only
//     when the profile does not already have it, so nothing a profile holds is
//     renamed or deleted and a value the user changed inside a profile wins.
//     The exception is SeedSyncFile (settings.json): the policy half of that
//     file — hooks, permissions, feature switches — is re-read from the default
//     on every pass, because a one-time copy meant the user's own configuration
//     stopped at the profile boundary with nothing on screen to say so. The
//     keys the vendor CLI writes itself stay the profile's, and a key only the
//     profile has is still never touched.
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
	// SeedSyncFile keeps a settings file in step with the default on every pass
	// instead of copying it once. The keys named in StateKeys belong to the
	// profile because the vendor CLI writes them itself (the chosen model, the
	// theme, the generated auto-mode body); every other top-level key is policy
	// the user maintains in one place (hooks, permissions, feature switches) and
	// is taken from the default. Used for Claude's settings.json and for Codex's
	// and Grok's config.toml, where the one-time copy meant a switch added to the
	// default configuration never reached a profile and nothing on screen said
	// so. JSON and TOML follow the same rules; the file's extension picks the
	// parser.
	SeedSyncFile
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
	// StateKeys names the top-level keys a SeedSyncFile entry leaves to the
	// profile — JSON keys in settings.json, tables and keys in config.toml.
	// Everything else in the file is policy and comes from the user's default
	// configuration on every pass.
	//
	// State is enumerated rather than policy on purpose: a state key left out
	// shows up as a UI setting that resets at launch, while a policy key left
	// out is a switch the user turned on in their default configuration that
	// never arrives and never says so.
	StateKeys []string
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
	// Synced holds the top-level key names a SeedSyncFile entry changed inside a
	// profile that already had the file. Key names only: the values are the
	// user's hooks, environment and permissions and never belong in a log.
	Synced []string
}

// Any reports whether the pass did anything worth logging.
func (r SeedResult) Any() bool {
	return len(r.Applied) > 0 || len(r.Failed) > 0 || len(r.Degraded) > 0 || len(r.Synced) > 0
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

// ApplyProfileSyncOverrides returns the adapter's entries adjusted by the sync
// settings one profile hand-wrote in config.yaml: settings_sync turns the
// launch-time sync off for that profile, profile_owned_keys adds keys the
// profile keeps for itself, and default_wins_keys hands keys back to the user's
// default configuration.
//
// **This is the only place those three settings are interpreted.** The sync
// (SeedProfileDirFor) and the report (`many-ai-cli doctor`) both go through it,
// because a report built from a different key list than the sync uses is worse
// than no report: it would name keys that are not going to move and stay quiet
// about ones that are.
//
// The adapter's own list is never modified — it is shared by every profile of
// that provider, and one profile bending the rules must not bend them for the
// next profile in the loop.
func ApplyProfileSyncOverrides(entries []SeedEntry, p config.SubscriptionProfile) []SeedEntry {
	syncOff := !p.IsSettingsSyncEnabled()
	defaultWins := trimmedKeySet(p.DefaultWinsKeys)
	if !syncOff && len(p.ProfileOwnedKeys) == 0 && len(defaultWins) == 0 {
		return entries
	}
	out := make([]SeedEntry, len(entries))
	copy(out, entries)
	for i := range out {
		if out[i].Kind != SeedSyncFile {
			continue
		}
		if syncOff {
			// Back to what the one-time copy always did: carried in when the
			// profile has nothing there, never touched afterwards. doctor's
			// drift row disappears with it, since a file that is not synced
			// cannot be out of step with anything.
			out[i].Kind = SeedCopyFile
			continue
		}
		out[i].StateKeys = effectiveStateKeys(out[i].StateKeys, p.ProfileOwnedKeys, defaultWins)
	}
	return out
}

// effectiveStateKeys returns StateKeys ∪ profile_owned_keys − default_wins_keys,
// blanks and duplicates dropped. The order is the adapter's own list first and
// the profile's additions as they were written, so the same config always
// produces the same list — the sync compares against what the previous pass
// wrote, and an unstable order would make a pass rewrite the file for nothing.
func effectiveStateKeys(base, owned []string, defaultWins map[string]bool) []string {
	out := make([]string, 0, len(base)+len(owned))
	seen := make(map[string]bool, len(base)+len(owned))
	for _, list := range [][]string{base, owned} {
		for _, key := range list {
			key = strings.TrimSpace(key)
			if key == "" || seen[key] || defaultWins[key] {
				continue
			}
			seen[key] = true
			out = append(out, key)
		}
	}
	return out
}

// trimmedKeySet turns a hand-written key list into a lookup set. Blank entries
// are dropped rather than matching a key named "": a stray dash in config.yaml
// must not quietly change which keys the sync takes over.
func trimmedKeySet(keys []string) map[string]bool {
	set := make(map[string]bool, len(keys))
	for _, key := range keys {
		if key = strings.TrimSpace(key); key != "" {
			set[key] = true
		}
	}
	return set
}

// seedTarget is one entry paired with where it lands and with whether the
// profile already holds something there.
type seedTarget struct {
	entry      SeedEntry
	dest       string
	destExists bool
}

// seedTargets returns every entry whose source exists, in adapter order, with
// the profile's own sync settings already applied.
func seedTargets(provider, profileDir string, p config.SubscriptionProfile) []seedTarget {
	if strings.TrimSpace(profileDir) == "" {
		return nil
	}
	var out []seedTarget
	for _, entry := range ApplyProfileSyncOverrides(seedEntriesFor(provider), p) {
		if entry.Source == "" || entry.Dest == "" {
			continue
		}
		if _, err := os.Lstat(entry.Source); err != nil {
			continue
		}
		dest := filepath.Join(profileDir, entry.Dest)
		// Lstat, not Stat: a dangling link still counts as "the profile has
		// something here", and replacing it is the user's call, not ours.
		_, err := os.Lstat(dest)
		out = append(out, seedTarget{entry: entry, dest: dest, destExists: err == nil})
	}
	return out
}

// PendingSeedEntries returns the entries whose source exists and whose
// destination does not, i.e. exactly what a seeding pass would carry in.
//
// `many-ai-cli doctor` calls this to report drift without changing anything:
// the seed only runs when the Hub prepares a profile, so a profile created
// before this existed, or one where the user later added a skill directory to
// their default configuration, stays visible instead of silently lagging.
//
// A SeedSyncFile entry is reported the same way — only when the profile has no
// copy at all — even though a seeding pass also re-syncs the copies that do
// exist. "Missing" keeps meaning missing here; whether the keys inside an
// existing file disagree with the default is a different question, answered by
// SyncDrift.
// The profile's own sync settings are not applied here on purpose: they change
// how an entry is kept in step, never whether the profile is missing it, and the
// answer to "missing" is the same either way.
func PendingSeedEntries(provider, profileDir string) []SeedEntry {
	var out []SeedEntry
	for _, target := range seedTargets(provider, profileDir, config.SubscriptionProfile{}) {
		if target.destExists {
			continue
		}
		out = append(out, target.entry)
	}
	return out
}

// SeedProfileDir carries every missing entry into profileDir and re-syncs the
// SeedSyncFile entries whether or not the profile already has them.
//
// It never returns an error. Each entry is independent, and a failure on one
// (an unreadable source, a filesystem that refuses links) must not stop the
// others or the session that asked for the profile.
//
// This is the standard-rules call. A profile that hand-wrote sync settings in
// config.yaml goes through SeedProfileDirFor.
func SeedProfileDir(provider, profileDir string) SeedResult {
	return SeedProfileDirFor(provider, profileDir, config.SubscriptionProfile{})
}

// SeedProfileDirFor is SeedProfileDir with one profile's own sync settings
// applied. A zero SubscriptionProfile means the standard rules, so for everyone
// who never opened config.yaml the two are the same call.
func SeedProfileDirFor(provider, profileDir string, p config.SubscriptionProfile) SeedResult {
	var result SeedResult
	for _, target := range seedTargets(provider, profileDir, p) {
		entry := target.entry
		// Every kind but SeedSyncFile is additive: what a profile already holds
		// is the user's and is never touched. SeedSyncFile is the deliberate
		// exception, because a hook or a permission added to the default
		// configuration has to reach a profile that was seeded months ago.
		if target.destExists && entry.Kind != SeedSyncFile {
			continue
		}
		var err error
		var degraded bool
		var synced []string
		switch entry.Kind {
		case SeedCopyFile:
			err = copySeedFile(entry.Source, target.dest)
		case SeedLinkDir:
			err = linkSeedDir(entry.Source, target.dest)
		case SeedJSONKeys:
			err = writeSeedJSONKeys(entry.Source, target.dest, entry.Keys)
		case SeedMirrorFile:
			degraded, err = mirrorSeedFile(entry.Source, target.dest)
		case SeedSyncFile:
			synced, err = syncSeedFile(entry.Source, target.dest, entry.StateKeys)
		default:
			continue
		}
		switch {
		case errors.Is(err, errNothingToSeed):
			// Source held none of the named keys. Nothing to report.
		case err != nil:
			result.Failed = append(result.Failed, entry.Dest)
		case target.destExists:
			// SeedSyncFile only, since every other kind was skipped above: the
			// profile already had the file, so this pass changed keys inside it
			// rather than carrying a file in.
			result.Synced = append(result.Synced, synced...)
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

// seedFileFormat is the part of a sync that depends on how the settings file is
// written: how to read one, how to tell two values apart, and how to write the
// merged document back.
//
// The rules themselves live in syncSeedObject and driftSeedObject and are shared
// by every format, because "the user's default owns the policy, the vendor CLI
// owns its own state" is a statement about configuration rather than about JSON.
// Claude keeps its settings in JSON and Codex and Grok keep theirs in TOML; a
// second copy of the rules for the second format is how the two would quietly
// start disagreeing about what a sync does.
type seedFileFormat[V any] struct {
	read   func(path string) (map[string]V, error)
	same   func(a, b V) bool
	render func(obj map[string]V) ([]byte, error)
}

// Values stay raw JSON: many-ai-cli never interprets what a hook or a permission
// means, and re-encoding through Go values would reorder and renumber them.
var jsonSeedFormat = seedFileFormat[json.RawMessage]{
	read:   readSeedJSONObject,
	same:   sameJSON,
	render: renderSeedJSON,
}

var tomlSeedFormat = seedFileFormat[any]{
	read:   readSeedTOMLObject,
	same:   reflect.DeepEqual,
	render: renderSeedTOML,
}

// isTOMLSeedFile picks the parser for one SeedSyncFile entry. The extension is
// the whole decision: both sides of a sync are the same vendor file in two
// places, so either name answers it and no adapter has to declare the format a
// second time next to a path that already ends in .toml.
func isTOMLSeedFile(paths ...string) bool {
	for _, path := range paths {
		if strings.EqualFold(filepath.Ext(path), ".toml") {
			return true
		}
	}
	return false
}

// syncSeedFile keeps one settings file in step with the user's default.
//
// The keys in stateKeys are the vendor CLI's own, so the profile's values are
// kept as they are. Every other top-level key is policy the user maintains in
// one place, so the default's value replaces the profile's — whole, not merged
// key by key, which is what makes a hook deleted from the default stop running
// in the profile as well. A key only the profile has is left alone, and a
// profile with no copy of the file yet is simply given the whole default, the
// same as SeedCopyFile always did.
//
// Two rules keep this from becoming a file this tool rewrites on every launch:
// nothing is written when the merge changes nothing, comparing values by what
// they mean rather than by their bytes; and a file neither side can parse is an
// error rather than a silent skip, because rewriting a settings file from a
// default we could not read is how a profile would lose its configuration.
func syncSeedFile(src, dst string, stateKeys []string) (synced []string, err error) {
	if _, statErr := os.Lstat(dst); statErr != nil {
		// Nothing to merge with. The first pass takes the default whole,
		// including the keys the CLI will take over from here on.
		return nil, copySeedFile(src, dst)
	}
	if isTOMLSeedFile(dst, src) {
		return syncSeedObject(src, dst, stateKeys, tomlSeedFormat)
	}
	return syncSeedObject(src, dst, stateKeys, jsonSeedFormat)
}

func syncSeedObject[V any](src, dst string, stateKeys []string, format seedFileFormat[V]) (synced []string, err error) {
	defaults, err := format.read(src)
	if err != nil {
		return nil, err
	}
	profile, err := format.read(dst)
	if err != nil {
		return nil, err
	}
	owned := stateKeySet(stateKeys)
	merged := make(map[string]V, len(profile)+len(defaults))
	for key, value := range profile {
		merged[key] = value
	}
	var changed []string
	for key, value := range defaults {
		if owned[key] {
			continue
		}
		if current, ok := profile[key]; ok && format.same(current, value) {
			continue
		}
		merged[key] = value
		changed = append(changed, key)
	}
	if len(changed) == 0 {
		return nil, nil
	}
	sort.Strings(changed)
	data, err := format.render(merged)
	if err != nil {
		return nil, err
	}
	if err := securefile.WriteAtomic(dst, data, 0o600); err != nil {
		return nil, err
	}
	return changed, nil
}

// SyncDrift reports what a SeedSyncFile entry would change, without touching
// anything: added names the policy keys the default has and the profile does
// not, changed names the ones both have with different values. Keys in
// stateKeys belong to the profile and are never reported.
//
// `many-ai-cli doctor` uses this to say that a profile disagrees with the
// user's default configuration. It returns key names only — no values, paths or
// hashes — so a report built from it stays safe to paste anywhere.
func SyncDrift(src, dst string, stateKeys []string) (added, changed []string, err error) {
	if isTOMLSeedFile(dst, src) {
		return driftSeedObject(src, dst, stateKeys, tomlSeedFormat)
	}
	return driftSeedObject(src, dst, stateKeys, jsonSeedFormat)
}

func driftSeedObject[V any](src, dst string, stateKeys []string, format seedFileFormat[V]) (added, changed []string, err error) {
	defaults, err := format.read(src)
	if err != nil {
		return nil, nil, err
	}
	profile, err := format.read(dst)
	if err != nil {
		return nil, nil, err
	}
	owned := stateKeySet(stateKeys)
	for key, value := range defaults {
		if owned[key] {
			continue
		}
		current, ok := profile[key]
		switch {
		case !ok:
			added = append(added, key)
		case !format.same(current, value):
			changed = append(changed, key)
		}
	}
	sort.Strings(added)
	sort.Strings(changed)
	return added, changed, nil
}

func stateKeySet(keys []string) map[string]bool {
	owned := make(map[string]bool, len(keys))
	for _, key := range keys {
		owned[key] = true
	}
	return owned
}

// readSeedSettingsFile reads one settings file, resolving a symlinked path to
// its target first. A default configuration that lives in a synced folder and is
// linked into place is an ordinary setup, and reading the link instead of the
// file behind it would make the default look empty — which, for a sync, means
// emptying the profile to match.
func readSeedSettingsFile(path string) ([]byte, error) {
	target := path
	if IsLinkedRuleFile(path) {
		resolved, err := filepath.EvalSymlinks(path)
		if err != nil {
			return nil, err
		}
		target = resolved
	}
	return os.ReadFile(target) // #nosec G304 -- path comes from an adapter's fixed entry list
}

// readSeedJSONObject reads one JSON object as a map of its top-level keys.
func readSeedJSONObject(path string) (map[string]json.RawMessage, error) {
	data, err := readSeedSettingsFile(path)
	if err != nil {
		return nil, err
	}
	var obj map[string]json.RawMessage
	if err := json.Unmarshal(data, &obj); err != nil {
		// Only the file's name, never its path or its contents: this error
		// reaches a log line and `many-ai-cli doctor`.
		return nil, fmt.Errorf("%s is not a JSON object: %w", filepath.Base(path), err)
	}
	return obj, nil
}

// readSeedTOMLObject reads one TOML document as a map of its top-level keys and
// tables, so that a config.toml merges under exactly the rules a settings.json
// does.
//
// Comments and the author's key order are lost here and are not recovered on the
// way out: the document is parsed into a map and written back from it. That is
// the price of merging TOML without hand-rolling a parser, and only the
// profile's copy pays it — the user's own default file is read and never
// written.
func readSeedTOMLObject(path string) (map[string]any, error) {
	data, err := readSeedSettingsFile(path)
	if err != nil {
		return nil, err
	}
	var obj map[string]any
	if err := toml.Unmarshal(data, &obj); err != nil {
		// Same rule as the JSON side: the file's name and nothing else. A TOML
		// parse error from this library carries a position, not the line's text.
		return nil, fmt.Errorf("%s is not a TOML document: %w", filepath.Base(path), err)
	}
	return obj, nil
}

// renderSeedJSON writes the merged object the way the vendor CLIs write theirs:
// two-space indentation and a trailing newline. Keys come out sorted, so the
// file is stable from one pass to the next, and HTML escaping is off so a hook
// command keeps its `>` and `&&` instead of turning into > and &.
func renderSeedJSON(obj map[string]json.RawMessage) ([]byte, error) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", "  ")
	if err := enc.Encode(obj); err != nil {
		return nil, err
	}
	return buf.Bytes(), nil
}

// renderSeedTOML writes the merged document back. go-toml sorts a map's keys,
// so the file is stable from one pass to the next and the next pass compares
// against exactly what this one wrote — the same property renderSeedJSON relies
// on. Nested tables and arrays of tables ([[hooks.Stop]]) are written in their
// own form rather than inlined, so the vendor CLI reads the same document it
// would have read from the default.
func renderSeedTOML(obj map[string]any) ([]byte, error) {
	return toml.Marshal(obj)
}

// sameJSON compares two raw values by what they mean rather than by their
// bytes, so that reindenting the default configuration, or writing its keys in
// another order, is not mistaken for a change the profile has to be rewritten
// for. The TOML side needs no equivalent: its values are already parsed into Go
// values, so reflect.DeepEqual compares meaning directly.
func sameJSON(a, b json.RawMessage) bool {
	var left, right any
	if err := json.Unmarshal(a, &left); err != nil {
		return false
	}
	if err := json.Unmarshal(b, &right); err != nil {
		return false
	}
	return reflect.DeepEqual(left, right)
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
