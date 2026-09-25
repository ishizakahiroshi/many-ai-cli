package clitrust

import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
)

// This file ports how Claude Code itself turns a working folder into project
// keys (read from the JS embedded in Claude Code 2.1.282's claude.exe on
// 2026-09-25; the minified names are only for finding the code again):
//
//   - The current directory is realpathSync(process.cwd()) — the non-native
//     realpath, which resolves links but keeps the letter case it was given,
//     drive letter included (confirmed with Bun 1.3.14: a child started in
//     "c:\...\realdir\sub" sees exactly that).
//   - Answering "Yes, I trust this folder" saves the current project config
//     (Jd), whose key is Jlt(): the canonical git root when the folder is
//     inside git — the main repository's root for a linked worktree — and
//     the folder itself otherwise. "No, exit" writes nothing, so a
//     hasTrustDialogAccepted:false entry is never an answer; it is the default
//     every new project entry starts with (Fae).
//   - Every key goes through lF: path.normalize, then "\" → "/" on Windows.
//   - Trust is projects[Jlt()].hasTrustDialogAccepted, or the same flag on the
//     folder or any parent up to the git root (the filesystem root outside
//     git) (pI → bb → Sb).
//
// Not ported (recorded in the C10 child plan's 判断ログ): resolving a
// junction on Windows (Go's filepath.EvalSymlinks no longer does, and the
// case-preserving walk Bun's realpath does is not reproduced here), and the
// exact guards Claude applies to odd gitdir contents. Where the port and
// Claude disagree, the key written is one Claude does not read and the child
// shows Claude's own trust prompt — the state before this package existed.

// claudePlan is what Claude Code itself would read and write for one working
// folder.
type claudePlan struct {
	// target is the folder Claude's project key names (see the file comment).
	target string
	// writeKey is Claude's project key for target (Jlt).
	writeKey string
	// walk lists the keys Claude checks after writeKey: the folder, then each
	// parent up to the git root (or the filesystem root outside git).
	walk []string
}

func claudePlanFor(dir string) claudePlan {
	cwd := claudeCurrentDir(dir)
	gitRoot, inGit := claudeGitRoot(cwd)
	target := cwd
	if inGit {
		target = claudeCanonicalRoot(gitRoot)
	}
	var walk []string
	for p := cwd; ; {
		walk = append(walk, claudeKey(p))
		if inGit && p == gitRoot {
			break
		}
		parent := filepath.Dir(p)
		if parent == p {
			break
		}
		p = parent
	}
	return claudePlan{target: target, writeKey: claudeKey(target), walk: walk}
}

// claudeCurrentDir is the folder as Claude's realpathSync(process.cwd())
// returns it, as far as it is ported: on Windows the cleaned absolute path
// (letter case untouched), elsewhere with symbolic links resolved.
func claudeCurrentDir(dir string) string {
	abs := cleanAbs(dir)
	if runtime.GOOS == "windows" {
		// filepath.EvalSymlinks would also rewrite the letter case of every
		// component (and uppercase the drive letter), which Claude's realpath
		// does not do.
		return abs
	}
	if real, err := filepath.EvalSymlinks(abs); err == nil {
		return real
	}
	return abs
}

// claudeKey mirrors lF, the one function every Claude project key goes
// through: path.normalize, then forward slashes on Windows. The drive letter
// and every other component keep the case they already had — Claude's own
// keys do too (a real ~/.claude.json was seen holding two projects entries
// that differ only in case, the shape of "C:/work/sampleApp" next to
// "C:/work/SampleApp"). An earlier version uppercased the drive letter, which
// only matched while the Hub happened to pass an uppercase one.
func claudeKey(path string) string {
	key := filepath.Clean(path)
	if runtime.GOOS == "windows" {
		key = strings.ReplaceAll(key, `\`, "/")
	}
	return key
}

// claudeGitRoot is Claude's find-git-root (qt): the nearest folder, starting
// with dir itself and ending with the filesystem root, that holds a ".git"
// directory or file.
func claudeGitRoot(dir string) (string, bool) {
	p := dir
	for {
		if info, err := os.Stat(filepath.Join(p, ".git")); err == nil && (info.IsDir() || info.Mode().IsRegular()) {
			return p, true
		}
		parent := filepath.Dir(p)
		if parent == p {
			return "", false
		}
		p = parent
	}
}

// claudeCanonicalRoot is Claude's canonical git root (Te → Jt): gitRoot
// itself, unless gitRoot is a linked worktree whose metadata points back at
// it, in which case the main repository's root (or, for a bare repository,
// its common directory). A submodule (no commondir) stays gitRoot.
func claudeCanonicalRoot(gitRoot string) string {
	data, err := os.ReadFile(filepath.Join(gitRoot, ".git"))
	if err != nil {
		// A .git directory cannot be read as a file — an ordinary checkout.
		return gitRoot
	}
	ref, ok := strings.CutPrefix(strings.TrimSpace(string(data)), "gitdir:")
	if !ok {
		return gitRoot
	}
	ref = strings.TrimSpace(ref)
	if ref == "" || unsupportedFolder(ref) {
		return gitRoot
	}
	worktreeGitDir := joinNative(gitRoot, ref)
	commondir, ok := readMetadataFile(filepath.Join(worktreeGitDir, "commondir"))
	if !ok {
		return gitRoot
	}
	commonRef := strings.TrimSpace(string(commondir))
	if commonRef == "" || unsupportedFolder(commonRef) {
		return gitRoot
	}
	commonDir := joinNative(worktreeGitDir, commonRef)
	if filepath.Dir(worktreeGitDir) != filepath.Join(commonDir, "worktrees") {
		return gitRoot
	}
	backlink, ok := readMetadataFile(filepath.Join(worktreeGitDir, "gitdir"))
	if !ok {
		return gitRoot
	}
	backRef := strings.TrimSpace(string(backlink))
	if backRef == "" || unsupportedFolder(backRef) {
		return gitRoot
	}
	realBack, errBack := canonicalPath(joinNative(worktreeGitDir, backRef))
	realRoot, errRoot := canonicalPath(gitRoot)
	if errBack != nil || errRoot != nil || realBack != filepath.Join(realRoot, ".git") {
		return gitRoot
	}
	if filepath.Base(commonDir) != ".git" {
		if pathExists(filepath.Join(commonDir, ".git")) {
			return gitRoot
		}
		return commonDir
	}
	return filepath.Dir(commonDir)
}
