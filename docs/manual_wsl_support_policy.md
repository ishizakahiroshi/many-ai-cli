# WSL Support Policy

> 最終更新: 2026-05-12(火) 12:07:50

## 短い説明

WSL 環境で `any-ai-cli` を使う場合は、WSL 内に Linux 版 `any-ai-cli` と対象 CLI（Claude Code / Codex CLI）をインストールし、WSL 内で完結して利用してください。

Windows 版 `any-ai-cli.exe` を WSL から呼び出す構成、Windows 側 Hub と WSL 側 CLI を混在させる構成、Windows 側 CLI と WSL 側 CLI を同じ Hub で混在させる構成はサポート対象外です。

理由は、Windows と WSL では `127.0.0.1`、PATH、ホームディレクトリ、PTY、ファイルパスの扱いが別環境として分かれるためです。混在構成は一部の環境で動く場合があっても、壊れ方が環境依存になりやすく、安定したサポートができません。

## サポート範囲

| 利用形態 | サポート | 説明 |
|---|---:|---|
| Windows 版 `any-ai-cli.exe` + Windows 側 Claude/Codex | 対象 | Windows ネイティブ環境として扱う |
| WSL 内 Linux 版 `any-ai-cli` + WSL 側 Claude/Codex | 対象 | Linux 環境として扱う |
| WSL から Windows 版 `any-ai-cli.exe` を呼ぶ | 対象外 | Windows/WSL 境界をまたぐため |
| Windows 側 Hub に WSL 側 wrapper/CLI を接続する | 対象外 | loopback とパスの扱いが環境依存になるため |
| Windows 側 CLI と WSL 側 CLI を同じ Hub で混在させる | 対象外 | セッションごとに実行環境が分裂するため |

## 推奨する案内文

WSL で利用したい場合は、WSL 内に Linux 版 `any-ai-cli` と Claude Code / Codex CLI をインストールし、WSL 内で `any-ai-cli claude` または `any-ai-cli codex` を実行してください。

Windows 側で利用したい場合は、Windows 版 `any-ai-cli.exe` と Windows 側にインストールした Claude Code / Codex CLI を使ってください。

Windows と WSL をまたいで同じ Hub に接続する構成は、現時点ではサポート対象外です。

## 詳細: なぜ混在構成をサポートしないか

### 1. `127.0.0.1` の意味が Windows と WSL で異なる

`any-ai-cli` の Hub は安全のため `127.0.0.1` にだけ bind します。これは外部公開しないための重要な制約です。

一方で、WSL 2 の通常 NAT 構成では、WSL 内から見た `127.0.0.1` は WSL 側の loopback です。Windows 側で起動している Hub の `127.0.0.1:<port>` と同じとは限りません。

Microsoft の WSL networking documentation でも、通常 NAT 構成で WSL から Windows 側サーバへ接続する場合は Windows ホスト IP を使う説明になっています。Windows 11 22H2 以降の mirrored networking では WSL から Windows 側の `127.0.0.1` に接続できる場合がありますが、これはユーザー環境の WSL 設定に依存します。

参照: https://learn.microsoft.com/en-us/windows/wsl/networking

`any-ai-cli` 側で混在構成を正式サポートするには、単に接続先 host を変えるだけでなく、セキュリティ制約、トークン、ブラウザ起動、UI API、ログ/添付ファイルのパス解決まで含めて設計し直す必要があります。

### 2. PATH が別環境になる

Windows 版 `any-ai-cli.exe` は Windows 側の PATH から `claude` / `codex` を探します。

WSL 内 Linux 版 `any-ai-cli` は WSL 側の PATH から `claude` / `codex` を探します。

そのため、WSL 内にだけ Claude Code / Codex CLI が入っているユーザーが Windows 版 `any-ai-cli.exe` を呼ぶと、対象 CLI が見つからない、または Windows 側の別 CLI が起動する可能性があります。

### 3. ホームディレクトリと設定ファイルが別になる

Windows 版は通常 `C:\Users\<user>\.any-ai-cli\config.yaml` を使います。

WSL 内 Linux 版は WSL の `~/.any-ai-cli/config.yaml` を使います。

同じユーザーに見えても、設定、トークン、ログ保存先、添付ファイル保存先は別物です。Windows 側 Hub と WSL 側 wrapper を混在させると、どちらの設定を正とするかが曖昧になります。

### 4. PTY 実装が異なる

Windows 版は ConPTY を使います。

Linux/WSL 版は Unix PTY を使います。

Claude Code / Codex CLI のような TUI は、PTY の改行、Enter、リサイズ、ANSI 制御、カーソル制御の差に影響を受けます。Windows/WSL をまたぐ構成では、どちらの PTY 前提で不具合を調査すべきかが曖昧になります。

### 5. ファイルパスが相互変換を必要とする

Windows パスは `C:\dev\project`、WSL パスは `/mnt/c/dev/project` や `/home/<user>/project` のように表現されます。

`any-ai-cli` はログ、添付ファイル、ディレクトリを開く操作、ターミナルを開く操作、UI からの spawn でローカルパスを扱います。混在構成をサポートするには、各 API で Windows パスと WSL パスの変換方針を定義する必要があります。

この変換は単純な文字列置換では不十分です。WSL distro ごとの rootfs、Windows から見えない Linux 側ファイル、Linux から見た Windows mount、権限、文字コード、ファイル選択 UI の違いを考慮する必要があります。

## 将来サポートする場合に必要な設計

Windows/WSL 混在を正式サポートするなら、少なくとも以下が必要です。

- Hub の接続先 host を `127.0.0.1` 固定ではなく、明示設定できるようにする
- WSL 内から Windows ホストを検出する方法を定義する
- mirrored networking と NAT の違いを検出またはユーザー設定で切り替える
- Windows パスと WSL パスの変換ルールを API ごとに決める
- Windows 側セッションと WSL 側セッションを UI 上で区別する
- 設定ファイル、トークン、ログ、添付ファイルの保存先をどちらに置くか決める
- Windows ConPTY と Unix PTY の差をテスト対象として追加する
- Windows 側ブラウザ起動、WSL 側ブラウザ起動、ファイルオープン操作の責務を整理する

これは単なる動作確認ではなく、クロス環境連携機能として別途設計する規模の対応です。

## 現時点の結論

WSL は「Linux 環境として WSL 内で完結する使い方」はサポート対象にできます。

ただし、Windows と WSL をまたぐ混在構成はサポート対象外とします。

ユーザーには「Windows で使うなら Windows 内で完結、WSL で使うなら WSL 内で完結」と案内してください。
