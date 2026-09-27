package clitrust

import (
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"strings"
)

// This file ports how codex itself turns a working folder into project keys
// (codex-rs at tag rust-v0.156.1, the version the parent plan's T3/T5 traces
// ran against):
//
//   - The trust prompt's "Trust and continue" writes
//     project_trust_key(trust_target), where trust_target is
//     resolve_root_git_project_for_trust(cwd) — the repository root, or for a
//     linked worktree the main repository's root — and cwd outside git
//     (tui/src/onboarding/directory_trust.rs, core/src/config/mod.rs
//     set_project_trust_level_inner).
//   - project_trust_key canonicalizes the path (dunce::canonicalize) and, on
//     Windows only, ASCII-lowercases it (config/src/loader/mod.rs).
//   - get_active_project looks up the folder first (canonical spelling, then
//     as given), then the repository root; each lookup tries the exact key,
//     then any key that lowercases to it (config/src/config_toml.rs).

// codexPlan is what codex itself would read and write for one working folder.
type codexPlan struct {
	// target is codex's trust target for the folder (see the file comment).
	target string
	// writeKey is the key codex writes for target.
	writeKey string
	// lookup lists the keys get_active_project tries, in its order.
	lookup []string
	// guard holds comparable spellings of the folder and of target. An
	// "untrusted" entry at or above any of them blocks the write, even where
	// codex's own lookup would not reach it (Q4a of the v0.9 review: the user
	// asked that no earlier "don't trust" above the folder be outvoted).
	guard []string
}

func codexPlanFor(dir string) codexPlan {
	cwd := cleanAbs(dir)
	target := cwd
	if root, ok := codexTrustRoot(cwd); ok {
		target = root
	}
	lookup := codexLookupKeys(cwd)
	if !samePathString(cleanAbs(target), cwd) {
		lookup = appendUnique(lookup, codexLookupKeys(target)...)
	}
	var guard []string
	for _, p := range []string{cwd, canonicalOrClean(cwd), target, canonicalOrClean(target)} {
		guard = appendUnique(guard, codexComparable(p))
	}
	return codexPlan{target: target, writeKey: codexKey(target), lookup: lookup, guard: guard}
}

// codexKey is project_trust_key: the canonical spelling, ASCII-lowercased on
// Windows. When the path cannot be canonicalized (it does not exist), codex
// falls back to the path as given; so does this.
func codexKey(path string) string {
	return codexNormalizeKey(canonicalOrClean(path))
}

// codexLookupKeys is normalized_project_lookup_keys: the canonical spelling,
// then the spelling as given when that differs.
func codexLookupKeys(path string) []string {
	return appendUnique(nil, codexKey(path), codexNormalizeKey(cleanAbs(path)))
}

// codexNormalizeKey is normalize_project_lookup_key.
func codexNormalizeKey(key string) string {
	if runtime.GOOS == "windows" {
		return asciiLower(key)
	}
	return key
}

// codexComparable puts a projects key (codex's, or one a user wrote by hand
// with "/" or a trailing separator) and a folder into one form, so the
// untrusted guard can compare them: cleaned, without a \\?\ drive prefix,
// ASCII-lowercased on Windows.
func codexComparable(key string) string {
	if runtime.GOOS == "windows" {
		if rest, ok := strings.CutPrefix(key, `\\?\`); ok && len(rest) >= 2 && rest[1] == ':' {
			key = rest
		}
	}
	return codexNormalizeKey(filepath.Clean(key))
}

func canonicalOrClean(path string) string {
	if canonical, err := canonicalPath(path); err == nil {
		return canonical
	}
	return cleanAbs(path)
}

// codexLookupEntry is project_config_for_lookup_key applied to keys in order:
// the exact key first, then the alphabetically first key that normalizes to
// it. It returns the key that matched.
func codexLookupEntry(projects map[string]codexProjectEntry, keys []string) (string, codexProjectEntry, bool) {
	for _, key := range keys {
		if entry, ok := projects[key]; ok {
			return key, entry, true
		}
		var matches []string
		for candidate := range projects {
			if codexNormalizeKey(candidate) == key {
				matches = append(matches, candidate)
			}
		}
		if len(matches) > 0 {
			sort.Strings(matches)
			return matches[0], projects[matches[0]], true
		}
	}
	return "", codexProjectEntry{}, false
}

// codexUntrustedAtOrAbove returns the first (alphabetically) entry marked
// untrusted whose folder is one of guard or a folder above one of them.
func codexUntrustedAtOrAbove(projects map[string]codexProjectEntry, guard []string) (string, bool) {
	var hits []string
	for key, entry := range projects {
		if entry.TrustLevel != "untrusted" {
			continue
		}
		folder := codexComparable(key)
		for _, g := range guard {
			if isSameOrBelow(g, folder) {
				hits = append(hits, key)
				break
			}
		}
	}
	if len(hits) == 0 {
		return "", false
	}
	sort.Strings(hits)
	return hits[0], true
}

// isSameOrBelow reports whether path is folder or lies inside it. Both are
// cleaned; a root folder already ends with its separator.
func isSameOrBelow(path, folder string) bool {
	if path == folder {
		return true
	}
	sep := string(filepath.Separator)
	if !strings.HasSuffix(folder, sep) {
		folder += sep
	}
	return strings.HasPrefix(path, folder)
}

// codexMaxGitMetadataBytes is MAX_GIT_METADATA_FILE_BYTES.
const codexMaxGitMetadataBytes = 64 * 1024

// codexTrustRoot ports resolve_root_git_project_for_trust
// (codex-rs/git-utils/src/trust.rs). It returns the repository root for a
// folder inside an ordinary checkout, the main repository's root for a folder
// inside a linked worktree whose metadata checks out, and false otherwise
// (outside git, a submodule, a worktree whose metadata does not match).
func codexTrustRoot(cwd string) (string, bool) {
	base := cwd
	if info, err := os.Stat(cwd); err != nil || !info.IsDir() {
		base = filepath.Dir(cwd)
	}
	var repoRoot string
	for {
		candidate, ok := nearestAncestorWithGit(base)
		if !ok {
			return "", false
		}
		dotGit := filepath.Join(candidate, ".git")
		info, err := os.Stat(dotGit)
		if err != nil {
			return "", false
		}
		// A .git directory without HEAD is not a repository; keep looking
		// above it.
		if !info.IsDir() || pathExists(filepath.Join(dotGit, "HEAD")) {
			repoRoot = candidate
			break
		}
		parent := filepath.Dir(candidate)
		if parent == candidate {
			return "", false
		}
		base = parent
	}

	dotGit := filepath.Join(repoRoot, ".git")
	info, err := os.Stat(dotGit)
	if err != nil {
		return "", false
	}
	if info.IsDir() {
		return repoRoot, true
	}
	if !info.Mode().IsRegular() || isSymlink(dotGit) || info.Size() > codexMaxGitMetadataBytes {
		return "", false
	}

	gitDir, ok := readGitdirFile(dotGit)
	if !ok {
		return "", false
	}
	if gitDirInfo, err := os.Stat(gitDir); err != nil || !gitDirInfo.IsDir() || isSymlink(gitDir) {
		return "", false
	}
	canonicalGitDir, err := canonicalPath(gitDir)
	if err != nil {
		return "", false
	}
	worktreesDir := filepath.Dir(canonicalGitDir)
	if filepath.Base(worktreesDir) != "worktrees" {
		return "", false
	}
	commonDir := filepath.Dir(worktreesDir)
	backlink, ok := readMetadataFile(filepath.Join(canonicalGitDir, "gitdir"))
	if !ok || strings.TrimSpace(string(backlink)) == "" {
		return "", false
	}
	worktreeDotGit := joinNative(canonicalGitDir, strings.TrimSpace(string(backlink)))
	if filepath.Base(worktreeDotGit) != ".git" {
		return "", false
	}
	commondir, ok := readMetadataFile(filepath.Join(canonicalGitDir, "commondir"))
	if !ok || strings.TrimSpace(string(commondir)) == "" {
		return "", false
	}
	linkedCommonDir := joinNative(canonicalGitDir, strings.TrimSpace(string(commondir)))
	registered, errRegistered := canonicalPath(filepath.Dir(worktreeDotGit))
	checkout, errCheckout := canonicalPath(repoRoot)
	linkedCommon, errLinked := canonicalPath(linkedCommonDir)
	if errRegistered != nil || errCheckout != nil || errLinked != nil ||
		registered != checkout || linkedCommon != commonDir {
		return "", false
	}

	// The main checkout owns the common directory: its .git is that
	// directory, or (--separate-git-dir) a gitdir pointer to it.
	mainRoot := filepath.Dir(filepath.Dir(filepath.Dir(gitDir)))
	mainDotGit := filepath.Join(mainRoot, ".git")
	mainInfo, err := os.Stat(mainDotGit)
	if err != nil {
		return "", false
	}
	mainGitDir := mainDotGit
	if !mainInfo.IsDir() {
		if mainGitDir, ok = readGitdirFile(mainDotGit); !ok {
			return "", false
		}
	}
	if canonicalMain, err := canonicalPath(mainGitDir); err != nil || canonicalMain != commonDir {
		return "", false
	}
	return mainRoot, true
}

// nearestAncestorWithGit is find_nearest_native_ancestor_with_markers with
// the single marker ".git": base itself, then each parent.
func nearestAncestorWithGit(base string) (string, bool) {
	p := base
	for {
		if pathExists(filepath.Join(p, ".git")) {
			return p, true
		}
		parent := filepath.Dir(p)
		if parent == p {
			return "", false
		}
		p = parent
	}
}

// readGitdirFile reads a "gitdir: <path>" pointer file and resolves the path
// against the file's folder.
func readGitdirFile(path string) (string, bool) {
	data, ok := readMetadataFile(path)
	if !ok {
		return "", false
	}
	target, ok := strings.CutPrefix(strings.TrimSpace(string(data)), "gitdir:")
	if !ok {
		return "", false
	}
	target = strings.TrimSpace(target)
	if target == "" {
		return "", false
	}
	return joinNative(filepath.Dir(path), target), true
}

// readMetadataFile reads a small git metadata file, refusing a symbolic link,
// anything that is not a regular file, and anything over 64 KiB.
//
// gosec G703: path is the approved child's working folder or a file its git
// metadata points to — the same files Codex reads to decide trust. They are
// only read, never written, and the result only picks which folder's entry
// Grant looks at.
func readMetadataFile(path string) ([]byte, bool) {
	info, err := os.Stat(path) // #nosec G703 -- git metadata of the approved working folder, read only (see above).
	if err != nil || !info.Mode().IsRegular() || isSymlink(path) || info.Size() > codexMaxGitMetadataBytes {
		return nil, false
	}
	data, err := os.ReadFile(path) // #nosec G703 -- regular file of at most 64 KiB, checked just above; read only.
	if err != nil || len(data) > codexMaxGitMetadataBytes {
		return nil, false
	}
	return data, true
}

// joinNative joins a path read from a git metadata file onto base; an
// absolute path replaces base, as Rust's Path::join does.
func joinNative(base, p string) string {
	if filepath.IsAbs(p) {
		return filepath.Clean(p)
	}
	return filepath.Join(base, p)
}

func pathExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}

func isSymlink(path string) bool {
	info, err := os.Lstat(path) // #nosec G703 -- only reads the file mode of a path under the approved working folder or its git metadata.
	return err == nil && info.Mode()&os.ModeSymlink != 0
}

func appendUnique(list []string, items ...string) []string {
	for _, item := range items {
		found := false
		for _, existing := range list {
			if existing == item {
				found = true
				break
			}
		}
		if !found {
			list = append(list, item)
		}
	}
	return list
}
