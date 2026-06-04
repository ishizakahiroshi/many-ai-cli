#!/bin/sh
# any-ai-cli イメージ自動更新スクリプト（cron から日次実行）
#
# サーバー配置先: /opt/any-ai-cli/aac-update.sh（compose.yaml と同じディレクトリ必須 —
# 自身の場所を compose プロジェクトディレクトリとして使うため）
#
# cron 登録例（root crontab。毎日 04:30 に実行）:
#   30 4 * * * /opt/any-ai-cli/aac-update.sh >> /var/log/aac-update.log 2>&1
#
# 開発中など自動更新を止めたいときは同じディレクトリに HOLD ファイルを置く:
#   touch /opt/any-ai-cli/HOLD   # 凍結
#   rm /opt/any-ai-cli/HOLD      # 再開（次回 cron から latest へ復帰）
set -eu

COMPOSE_DIR="$(cd "$(dirname "$0")" && pwd)"
HOLD_FILE="$COMPOSE_DIR/HOLD"

log() {
  printf '%s aac-update: %s\n' "$(date '+%Y-%m-%dT%H:%M:%S%z')" "$*"
}

if [ -f "$HOLD_FILE" ]; then
  log "HOLD file exists; skipping update ($HOLD_FILE)"
  exit 0
fi

cd "$COMPOSE_DIR"

log "pulling images"
docker compose pull --quiet

# up -d はイメージが変わったコンテナだけ再作成する（変化なしなら無停止）
log "recreating containers if image changed"
docker compose up -d

# 旧イメージの残骸を回収（タグなしの dangling のみ。:dev 等の名前付きは消さない）
docker image prune -f >/dev/null

log "done"
