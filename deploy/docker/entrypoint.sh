#!/bin/sh
# any-ai-cli コンテナ entrypoint: Hub と socat 中継を併走させる。
#
# 必須環境変数:
#   HUB_PORT — ユーザー割当ポート。ブラウザ URL・host 側 publish ポートと一致させること。
#              Hub の Host/Origin 検証はポート完全一致を要求する
#              （internal/hub/http_helpers.go isAllowedHubHost）。
set -eu

HUB_PORT="${HUB_PORT:?HUB_PORT is required (per-user assigned port)}"
CFG_DIR="$HOME/.any-ai-cli"
CFG="$CFG_DIR/config.yaml"
WRAPPER_TERM_GRACE_SECONDS=20
WRAPPER_PATTERN='any-ai-cli (wrap|claude|codex|copilot|cursor-agent)( |$)'
SOCAT_LOOP_PID=""
HUB_PID=""
TERMINATING=0

log() {
  printf '%s\n' "aac-entrypoint: $*"
}

wrapper_running() {
  pgrep -f "$WRAPPER_PATTERN" >/dev/null 2>&1
}

stop_socat() {
  # 中継ループのサブシェルはフォアグラウンドの socat が終了するまで TERM trap を
  # 処理できない。ループへ TERM を予約 → socat 本体を kill してブロックを解く →
  # wait の順にする。socat より先に wait すると永久に戻らず（デッドロック）、
  # Hub 終了後も entrypoint が exit できずコンテナが残り続け、
  # restart: unless-stopped による自動復旧が効かなくなる。
  if [ -n "$SOCAT_LOOP_PID" ] && kill -0 "$SOCAT_LOOP_PID" 2>/dev/null; then
    log "stopping socat relay loop pid=$SOCAT_LOOP_PID"
    kill -TERM "$SOCAT_LOOP_PID" 2>/dev/null || true
    pkill -TERM -f 'socat TCP-LISTEN:48000' 2>/dev/null || true
    wait "$SOCAT_LOOP_PID" 2>/dev/null || true
  fi

  # ループが trap 処理前に socat を再起動した場合の取りこぼし掃除
  pkill -TERM -f 'socat TCP-LISTEN:48000' 2>/dev/null || true
}

wait_for_wrappers() {
  elapsed=0
  while [ "$elapsed" -lt "$WRAPPER_TERM_GRACE_SECONDS" ]; do
    if ! wrapper_running; then
      return 0
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  return 1
}

terminate_wrappers() {
  reason="$1"
  if wrapper_running; then
    log "$reason; sending TERM to wrapper processes"
    pkill -TERM -f "$WRAPPER_PATTERN" 2>/dev/null || true
    if wait_for_wrappers; then
      log "wrapper processes exited"
    else
      log "wrapper grace period expired after ${WRAPPER_TERM_GRACE_SECONDS}s"
    fi
  else
    log "no wrapper processes found"
  fi
}

on_term() {
  if [ "$TERMINATING" -eq 1 ]; then
    return
  fi
  TERMINATING=1

  terminate_wrappers "shutdown requested"

  stop_socat

  if [ -n "$HUB_PID" ] && kill -0 "$HUB_PID" 2>/dev/null; then
    log "sending TERM to Hub pid=$HUB_PID"
    kill -TERM "$HUB_PID" 2>/dev/null || true
    wait "$HUB_PID" 2>/dev/null || true
    log "Hub exited"
  fi

  exit 0
}

# 初回起動時のみ token を生成し、コンテナ向け設定で config.yaml を事前生成する。
# LoadOrCreate は「defaults を作ってから YAML を上書き unmarshal」するため、
# ここに書かないキーは実行時デフォルトのままになる（internal/config/config.go）。
# - open_browser: false … headless コンテナでブラウザ起動を試みない
# - auto_shutdown: false … Hub プロセスの自動終了を止め、常駐させる
# - idle_timeout_min: 0 … 「最後の UI 切断から 60 分で全 PTY セッションを kill」する
#   既定動作（server.go startIdleTimerLocked → killAllWrappers）を無効化する。
#   リモート運用ではトンネル切断・PC スリープで UI が落ちるのが常態のため、
#   放置で走行中の AI セッションが消されると実害になる（UI 設定から変更可能）
if [ ! -f "$CFG" ]; then
  mkdir -p "$CFG_DIR"
  chmod 700 "$CFG_DIR"
  TOKEN="$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')"
  cat > "$CFG" <<EOF
hub:
  port: $HUB_PORT
  open_browser: false
  auto_shutdown: false
  idle_timeout_min: 0
token: $TOKEN
EOF
  chmod 600 "$CFG"
fi

# docker exec の対話シェルで claude/codex が透過 wrap されるようにする（冪等追記）
if [ -f "$HOME/.bashrc" ] && ! grep -q 'any-ai-cli shell-init' "$HOME/.bashrc"; then
  {
    echo ''
    echo '# any-ai-cli transparent wrap'
    echo 'export ANY_AI_CLI_AUTO=1'
    echo 'eval "$(any-ai-cli shell-init)"'
  } >> "$HOME/.bashrc"
fi

mkdir -p "$HOME/work"

# socat: docker publish からの接続を 48000 で受け、loopback の Hub へ中継する。
# 0.0.0.0 bind はコンテナ境界の内側だけの話。host 側 publish は 127.0.0.1 限定
# （users/<user>.yaml の ports 定義）であり、外部公開にはならない。
# socat が落ちても Hub は生かしたまま自動再起動する。
(
  trap 'exit 0' TERM INT
  while :; do
    socat TCP-LISTEN:48000,fork,reuseaddr,bind=0.0.0.0 TCP:127.0.0.1:"$HUB_PORT" || true
    sleep 1
  done
) &
SOCAT_LOOP_PID=$!

trap 'on_term' TERM INT

# Hub を起動し、entrypoint 側で SIGTERM を受けて wrapper → Hub の順に止める。
# --port は config.yaml の値より優先され、Host 検証もこのポートで行われる。
any-ai-cli serve --port "$HUB_PORT" &
HUB_PID=$!
log "Hub started pid=$HUB_PID"

set +e
wait "$HUB_PID"
HUB_STATUS=$?
set -e

log "Hub exited with status $HUB_STATUS"
terminate_wrappers "Hub exited"
stop_socat
exit "$HUB_STATUS"
