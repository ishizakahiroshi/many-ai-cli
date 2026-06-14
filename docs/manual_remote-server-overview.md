# リモート/サーバー利用の入口（何を入れて、どの手順を使うか）

> 最終更新: 2026-06-14(日) 21:01:38

many-ai-cli は「自分の PC で AI を動かす人」と「リモートサーバーの AI を使う人」で **入れるものが違う**。このページは「自分は何を入れて、どの設定手順に進めばいいか」を 1 表で示す入口（索引）。個別の設定手順はリンク先の `manual_remote-server-agent-*.md` を参照。

## まず役割を3つに分ける

- **A. AI実行ホスト** — AI CLI が実際に走る側（`many-ai-cli serve`）。PTY とダッシュボードを持つ。＝ いわゆる「Hub」。
- **B. コネクタ（トンネル母艦）** — 別の A へ SSH/WSL トンネルを張って生かし続ける側（`many-ai-cli-launcher`。フル Hub にも `/api/servers` として内蔵）。＝ いわゆる「母艦」。
- **C. クライアント** — ただ見て操作するブラウザ / PWA / スマホ。常駐物なし。

同じマシンが A と B を兼ねられる（自分の AI を動かしつつリモートにも繋ぐ）。だから「自分が母艦にもハブにもなる」が起きる。下の表はそれを「**自分のマシンに何を入れるか**」へ還元したもの。

## 自分は何を入れる？ → どの手順へ

| あなたの使い方 | 自分のマシンに入れる | ロール | 常駐 | 進む手順 |
|---|---|---|---|---|
| 自分の PC で AI を動かす（リモート無し） | `many-ai-cli`（フル） | A | あり | README「Quick Start」 |
| 自分の PC でも動かす ＋ リモートにも繋ぐ | `many-ai-cli`（フル）だけ ※Bは内蔵 | A＋B | あり | README「Quick Start」＋下のリモート手順 |
| リモートの AI だけ・PC 経由・**エージェントで自動設定** | `many-ai-cli-launcher` | B | あり（最小） | [単一サーバー(pnpm)](manual_remote-server-agent-single.md) / [Docker](manual_remote-server-agent-docker.md) |
| リモートの AI だけ・PC 経由・**UI ボタンで手動取り込み** | `many-ai-cli-launcher` | B | あり（最小） | [plan_server-profile-export-import](local/plan_server-profile-export-import.md)（計画中） |
| リモートの AI だけ・**スマホ VPN 直結** | 何も入れない（PWA） | C | なし | Hub UI の「📱 モバイル接続」 |

※「両方使う人」は **フルを1個入れるだけ**。コネクタ B は `serve` に内蔵（`/api/servers`）なので、launcher を別途入れる必要はない。
※「繋がれる側」のサーバーには別途 A（`many-ai-cli serve`）が入っている前提。**その設置こそが下の2つの設定手順**。

> **原則：サーバー利用は `many-ai-cli-launcher` 単体が正道。** リモートの AI だけ使うなら、ローカルにフル Hub（`serve`）を入れる必要はない。launcher が SSH トンネルを張ってリモート Hub の画面を開くだけで完結する。フル Hub に内蔵されたコネクタ（`/api/servers`）は「自分でも AI を動かしつつ、ついでにリモートにも繋ぐ人」向けのおまけと捉える（＝ server-only の人にフル Hub を強いない）。

## リモート設定手順の選び方（serve / tunnel）

「サーバーの AI を使う」人がエージェントで自動設定する場合、手順は2つ。違いは **接続技術ではなく「Hub がもう動いているか」**：

| | [単一サーバー（pnpm）](manual_remote-server-agent-single.md) | [Docker](manual_remote-server-agent-docker.md) |
|---|---|---|
| リモート導入 | pnpm global install | GHCR イメージを compose で pull |
| Hub の起動 | `serve`（都度 or systemd / tmux / nohup で常駐） | `restart: unless-stopped` で常駐 |
| launcher モード | `serve`（トークン不要） | `tunnel`（`token_command` で自動取得） |
| 向く人 | 単一ユーザー・隔離不要（**物理1台でも可**） | 複数ユーザー隔離・自動更新を仕組み化 |

> **「常駐」は Docker 専用ではない。** 物理 Linux 1台でも `serve` を systemd / tmux で常駐させ、launcher を tunnel モードにすれば同じ運用ができる（その場合の `token_command` 例は[単一サーバー手順の「運用形態」節](manual_remote-server-agent-single.md#運用形態都度起動--常駐)を参照）。

## 関連

- 仕組み・トラブル対処: [manual_remote-server-ssh-tunnel.md](manual_remote-server-ssh-tunnel.md)
- launcher 全般（英語・配布物の入手含む）: README「Unified launcher」節
- エージェント無し派の UI 取り込み（計画）: [local/plan_server-profile-export-import.md](local/plan_server-profile-export-import.md)
