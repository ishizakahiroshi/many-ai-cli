# v0.1.4 リリース時のブランチ・タグ手順

> 最終更新: 2026-05-13(水) 00:49:39

このメモは、`develop` で回収中の作業を `v0.1.4` として出すときの補助手順。
通常のリリース手順は `docs/manual_release.md` を正本とし、このファイルは今回の
`develop` / `main` / tag の置き方だけを扱う。

## 現状の前提

2026-05-13 時点の確認では、`v0.1.3` タグは `main` 側の merge commit に付いている。
一方、作業中の `develop` は `v0.1.3` タグの commit そのものを履歴に含まず、
`v0.1.2` 側から進んでいる。

この状態で `develop` 上の dev build が近傍タグを解決すると、表示が `v0.1.2`
由来になることがある。`v0.1.4` リリース前に、まず `develop` へ `main` を取り込む。

## 推奨方針

`develop` に `main` を取り込んだあと、`develop` 上でリリース準備 commit を作る。
その commit に `v0.1.4` tag を付け、同じ commit を `main` へ fast-forward merge する。

この方式なら、次が一致する。

- `v0.1.4` tag が指す commit
- `main` のリリース commit
- GitHub Actions の Release workflow がビルドする commit

## 事前確認

Claude / Codex など別プロセスが作業中なら、先にそれらの変更を落ち着かせる。
未コミット変更が残っている状態で tag を打たない。

```powershell
git status --short
git branch --show-current
git tag --list --sort=-v:refname
git log --oneline --decorate --graph --max-count=20 --all
```

期待する状態:

- 作業ブランチは `develop`
- リリースに含める変更だけが commit 済み、またはこれから commit する対象として明確
- `v0.1.4` tag はまだ存在しない

## C1: develop に main を取り込む

`develop` で作業する。

```powershell
git checkout develop
git fetch origin
git merge main
```

conflict が出た場合は、リリース対象の実装を残す形で解消する。
解消後に test を通す。

```powershell
go test ./...
```

ここで `git describe` が `v0.1.3-...` 系になることを確認する。

```powershell
git describe --tags --always --dirty
```

## C2: v0.1.4 リリース準備 commit を作る

必要に応じて、通常のリリース手順に従って手動 bump 対象を更新する。
詳細は `docs/manual_release.md` の「タグを打つ前の確認」と
「CHANGELOG.md の更新」を参照。

主な確認対象:

- `CHANGELOG.md`
- `winres/winres.json`
- `cmd/any-ai-cli/rsrc_windows_*.syso`
- `README.md`
- `README.ja.md`
- `docs/v0.1.x-any-ai-cli-design.md`
- `CLAUDE.md`

検証する。

```powershell
go test ./...
go vet ./...
go mod verify
```

必要なら third-party notice も確認する。

```powershell
.\scripts\local\check-third-party.ps1
```

リリース準備 commit を作る。

```powershell
git status --short
git add .
git commit -m "chore: v0.1.4 リリース準備"
```

## C3: develop 上のリリース commit に tag を付ける

作業ツリーが clean であることを確認する。

```powershell
git status --short
```

`v0.1.4` tag を作る。

```powershell
git tag v0.1.4
git show --stat v0.1.4
git describe --tags --always --dirty
```

期待する状態:

- `git describe --tags --always --dirty` が `v0.1.4` を返す
- `-dirty` が付かない
- `git show --stat v0.1.4` の内容がリリース対象 commit と一致する

## C4: tag 付き commit を main に入れる

`main` を `develop` の tag 付き commit へ fast-forward する。

```powershell
git checkout main
git merge --ff-only develop
```

`--ff-only` が失敗する場合は、`main` と `develop` が単純な前後関係ではない。
その場合は無理に進めず、`git log --oneline --decorate --graph --max-count=30 --all`
で履歴を確認してから判断する。

確認する。

```powershell
git show --no-patch --decorate --oneline HEAD
git show --no-patch --decorate --oneline v0.1.4
```

期待する状態:

- `HEAD` と `v0.1.4` が同じ commit
- `main` が `v0.1.4` tag 付き commit を指している

## C5: push して Release workflow を起動する

`main` を push したあと、tag を push する。

```powershell
git push origin main
git push origin v0.1.4
```

tag push により Release workflow が起動する。
以降は `docs/manual_release.md` の「GitHub Actions 確認」と
「リリース後確認」に従う。

```powershell
gh run list --repo ishizakahiroshi/any-ai-cli --workflow=Release --limit 3
```

## 代替方針: main の merge commit に tag を付ける場合

fast-forward ではなく、`main` に merge commit を作る運用にする場合は、
`develop` ではなく `main` へ merge した後の commit に tag を付ける。

```powershell
git checkout main
git merge develop
git tag v0.1.4
git push origin main
git push origin v0.1.4
```

この場合も、tag が付いた commit と GitHub Releases でビルドされる commit が
一致していることを確認する。

## 避けること

- dirty な作業ツリーで tag を打つ
- `develop` に `main` を取り込まないまま `v0.1.4` tag を打つ
- tag を付けた後に追加修正 commit を作り、そのまま同じ tag で release する
- squash merge で `main` に入れて、tag 付き commit と `main` の commit を分離する
- 公開後の tag を force push して差し替える
