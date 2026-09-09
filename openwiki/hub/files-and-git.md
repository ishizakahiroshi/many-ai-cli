---
type: architecture-component
title: Files and Git Tabs
description: The Hub's file-browsing/editing HTTP endpoints — their read-scope trust model, atomic no-clobber renames, and secure on-disk permissions — plus the Git tab's argv-only git invocation.
tags: [files, git, atomic-write, securefile, toctou, commit, hub-api]
verified:
  - by: openwiki/0.5.0
    at: 2026-09-08T13:17:25.310Z
sources:
  - id: openwiki-source-5d455ef6d1025eb932eccd15
    resource: repo://internal/hub/atomic_rename.go
  - id: openwiki-source-10cfa41cfbda7424f4077223
    resource: repo://internal/hub/files_content.go
  - id: openwiki-source-f8bc719bcdc0132ed82e81c1
    resource: repo://internal/hub/files_save.go
  - id: openwiki-source-179d22e75f389aee04013103
    resource: repo://internal/hub/files_scope.go
  - id: openwiki-source-4e980d4ab6b7e2b131d0bce6
    resource: repo://internal/hub/git_commit.go
generated: { by: "claude-code", at: "2026-09-08T13:17:25.310Z" }
---

## Files API: read scope depends on how the caller connects, writes never do

`filesScopeRestricted` (`files_scope.go`) is the one function deciding whether a *read* request is confined to an allowed root (cwd / git root / attachments / orchestration directories, plus a chat-mention fallback) or can read anywhere. Its own comment lays out the reasoning: `many-ai-cli` is a single-user tool running on the Hub host, and a browser connecting over direct loopback is treated as equivalent to the OS user themself — that user can already open any file with Explorer, and the wrapped AI CLI processes run with the same OS permissions and can already read anything, so confining only the Hub's own reads to cwd/git-root would not be a real security boundary for a direct-loopback caller (reinforced by the fact that `POST /api/spawn` already accepts `provider="shell"`, and `spawnCwdTooBroad` only rejects a drive root or the home directory itself — a token holder already has an equivalent, broader path available). A **logically remote** caller — Tailscale `serve`, a `trusted_networks` peer, a phone — is different: the operator there is not necessarily the OS user, so those reads stay confined to the allowed roots, exactly the same direct-loopback/logically-remote split `POST /api/list-subdirs` (`misc_handlers.go`'s `listSubdirsAllowedRemote`) already uses.

**Writes are never governed by this function.** `files-save`/`files-create`/mkdir/move/rename/delete stay confined to cwd/git-root regardless of connection kind, because the practical risk there is different: accidentally corrupting something outside the user's own repository, not a confidentiality boundary. Even within the allowed-roots read path, `secretReadDeniedExtensions` (`.pem`, `.key`) and `secretReadDeniedBasenames` are blocked outright.

`/api/files-content` additionally enforces a fixed allowlist of previewable text extensions (`previewableTextExtensions`) and a 1 MiB size cap (`filesContentMaxSize`); its response's `ReadOnly` flag distinguishes a normal in-scope read from one permitted only because the path was mentioned in chat and fell back to read-only access outside the allowed roots.

## Save with conflict detection

`POST /api/files-save` (`files_save.go`) accepts an optional `baseMtime` (RFC3339) alongside the path and new content; its documented validation order is token+method, absolute-path + allowed-root scope check, extension allowlist, a 1 MiB content size cap, confirming the target exists and is not a directory (this endpoint never creates a new file), and finally — only if `baseMtime` was supplied — comparing it against the file's current on-disk mtime and returning `409 Conflict` on a mismatch, which is the mechanism behind the "save with conflict detection" feature: a second editor's write since the client last read the file is caught rather than silently overwritten. The request body itself is capped separately at `filesSaveBodyMaxBytes` (2 MiB, JSON overhead included) ahead of the narrower content-size check.

## No-clobber rename: closing a TOCTOU

`atomicRenameNoReplace` (`atomic_rename.go`) exists to fix a specific defect (tracked as `HUB-5`): a plain `os.Rename` (POSIX `rename(2)` / Windows `MoveFileEx` with `REPLACE_EXISTING`) succeeds even when the destination already exists, so `files_move.go`/`files_rename.go` checking "does the destination already exist?" and then calling a plain rename left a time-of-check-to-time-of-use gap in which another writer could create the destination in between, and the second rename would silently overwrite it. The fix is OS-specific "fail if destination exists" primitives — `unix.Renameat2(..., RENAME_NOREPLACE)` on Linux, `windows.MoveFileEx` called *without* `REPLACE_EXISTING`, and a fallback for other OSes — so the existence check and the rename itself become one atomic operation instead of two separate steps a concurrent writer could race between. A caller can distinguish "failed because the destination already exists" (via `isRenameTargetExistsErr`) from other rename failures and map that specific case to an HTTP `409 Conflict`.

## Secure on-disk permissions beyond POSIX chmod

`internal/securefile` exists because `os.Chmod(path, 0o600)` does not do what its POSIX-permission-bits appearance suggests on Windows — Windows only interprets that call as toggling the `READONLY` file attribute, leaving the actual NTFS DACL (discretionary access control list) unrestricted, so a "0600" config file was still readable by other accounts on the same machine. `securefile.RestrictFile` is a small OS-abstracted package: on Windows it calls `SetNamedSecurityInfo` to explicitly restrict the DACL to the owning user plus `SYSTEM`/`Administrators`; on other OSes it is a no-op, since the existing `os.Chmod` semantics are already sufficient there. Every save path that writes a secret-bearing file — `config.yaml` (see [Configuration and On-Disk Layout](/openwiki/architecture/configuration.md)), `push_store.json`, `launcher-profiles.yaml`, and the approval-rules central directory — calls this immediately after writing; a failure here is logged as a warning rather than treated as fatal, since the write itself already succeeded and failing the whole operation over a permissions tightening step would risk leaving config unreadable.

## Git tab: `runGit` and why its gosec suppression is safe

Every Git-tab operation funnels through one helper, `runGit(ctx, cwd, args...)`, which always runs `git -C <cwd> -c core.quotePath=false <args...>` via `exec.CommandContext` — passing argv directly rather than through a shell, so there is no shell-injection surface regardless of what the arguments contain. `core.quotePath=false` is set specifically so non-ASCII paths (e.g., Japanese filenames) come back as raw UTF-8 in `name-status`/`numstat` output instead of git's default octal-escaped quoted form, which would otherwise break both the UI's display of those paths and any later pathspec reuse of the same string (a quoted path handed back to `git show <hash> -- "<quoted>"` matches nothing and silently returns an empty, exit-0 diff — a false-negative rather than a visible error).

The function carries a `#nosec G702` gosec suppression with a documented justification rather than a blanket ignore: gosec's taint analysis cannot follow the actual safety argument and its verdict was observed to flip between flagging 1 and 2 findings on the identical commit (2026-08-28), so the suppression exists to make CI deterministic rather than to silence a real risk. The safety argument the comment records, re-verified across every call site as of 2026-08-28: the one way a git argument could turn dangerous is being interpreted as an option flag (e.g. a value starting with `--upload-pack=`) rather than a literal, and every call site that passes a variable value already forecloses that — `git_log.go`'s ref and `git_show.go`'s hash both pass through `validRevision()`, which explicitly rejects any value starting with `-`; `git_diff.go`/`git_show.go` place file paths after a `--` separator; every other argument in every call site is a source-code literal; and `cwd` itself is always a session's own cwd on the local host, never externally supplied input. Any new call site that passes a variable value must preserve one of those four conditions for the suppression to remain valid.

## Commit all: stage-then-review

The `git-commit-all` flow (`git_commit.go`) enforces bounded inputs — a commit subject capped at `gitCommitSubjectMaxLen` (200 chars), a body capped at `gitCommitBodyMaxLen` (8192 chars), and the diff shown for review capped at `gitCommitDiffMaxBytes` (48 KiB) — backing the README's "stage all current working-tree changes and create a local commit after an explicit review step" feature: the API is shaped around confirming a subject/body against an already-computed diff rather than blindly committing whatever is staged.
