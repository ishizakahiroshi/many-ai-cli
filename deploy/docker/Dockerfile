# syntax=docker/dockerfile:1
# any-ai-cli マルチユーザー運用イメージ（1 ユーザー = 1 コンテナ）
#
# build context はリポジトリルート。
# 通常は GitHub Actions（.github/workflows/docker-image.yml）がビルドして
# ghcr.io/ishizakahiroshi/any-ai-cli へ publish し、VPS は pull するだけ
# （users/<user>.yaml の image: 参照と aac-update.sh の cron が更新を担う）。
# 開発中バイパス（VPS ローカルビルド）の例:
#   docker build -t ghcr.io/ishizakahiroshi/any-ai-cli:dev \
#     -f src/deploy/docker/Dockerfile src
#
# 設計の前提（docs/local/archive/plan_docker-multiuser-isolation.md）:
# - Hub はコンテナ内 loopback の $HUB_PORT で listen する（Host/Origin 検証が
#   ポート完全一致を要求するため、ユーザー割当ポートと一致させる）
# - 外向きの受け口は socat（0.0.0.0:48000 → 127.0.0.1:$HUB_PORT）で、
#   host 側 publish は 127.0.0.1 限定。外部公開はしない

# --- web build stage: TypeScript フロントを web/dist に生成 ---
# bun は依存取得のみに使い、ビルドスクリプト実行は node のまま（攻撃面を最小化）
FROM node:22-bookworm AS web-build
COPY --from=oven/bun:1.3.14 /usr/local/bin/bun /usr/local/bin/bun
WORKDIR /src/web
COPY web/package.json web/bun.lock web/bunfig.toml ./
RUN bun install --frozen-lockfile
COPY web ./
RUN bun run build

# --- build stage: any-ai-cli バイナリ（CGO 不要・web/dist は go:embed 同梱）---
FROM golang:1.25-bookworm AS build
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY . .
COPY --from=web-build /src/web/dist ./web/dist
RUN CGO_ENABLED=0 go build -trimpath -ldflags "-s -w" -o /out/any-ai-cli ./cmd/any-ai-cli

# --- runtime stage: Ubuntu 24.04 + Node.js 22 + provider CLI ---
FROM ubuntu:24.04
ARG CURSOR_AGENT_VERSION=2026.06.03-0bbb28e
ARG TARGETARCH
ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8

# git: Hub の Git 機能・provider CLI が実行時に必須
# socat: Hub(loopback) への中継。curl/ca-certificates: GitHub からの定義取得
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl git socat openssh-client procps less nano jq \
 && curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
 && apt-get install -y --no-install-recommends nodejs \
 && npm install -g \
      @anthropic-ai/claude-code@2.1.162 \
      @openai/codex@0.137.0 \
      @github/copilot@1.0.59 \
 && apt-get clean && rm -rf /var/lib/apt/lists/*
# ↑ provider CLI はバージョン固定（動作確認済みの組で再ビルドを再現可能にする）。
#   更新は「pin を上げる → 再ビルド → 承認 action-bar の検出を確認 → 全コンテナ再作成」の手順で一斉に行う。
#   コンテナ内での self-update / 手動 update は使わない（~/.local へ入ると volume に残留し
#   イメージ更新が効かない「亡霊バージョン」になる。詳細は manual の運用節）

# cursor-agent: 公式インストーラは「最新」固定のため、インストーラが参照する
# versioned package URL を直接取得して pin する。/home/ubuntu は実行時に volume で
# 覆われるため、全ユーザー共通の /opt へ配置する。
RUN set -eu; \
    case "${TARGETARCH:-amd64}" in \
      amd64) CURSOR_AGENT_ARCH="x64" ;; \
      arm64) CURSOR_AGENT_ARCH="arm64" ;; \
      *) echo "unsupported TARGETARCH for cursor-agent: ${TARGETARCH:-unset}" >&2; exit 1 ;; \
    esac; \
    CURSOR_AGENT_DIR="/opt/cursor-agent/versions/${CURSOR_AGENT_VERSION}"; \
    mkdir -p "$CURSOR_AGENT_DIR"; \
    curl -fSL "https://downloads.cursor.com/lab/${CURSOR_AGENT_VERSION}/linux/${CURSOR_AGENT_ARCH}/agent-cli-package.tar.gz" \
      | tar --strip-components=1 -xzf - -C "$CURSOR_AGENT_DIR"; \
    ln -sf "$CURSOR_AGENT_DIR/cursor-agent" /usr/local/bin/cursor-agent; \
    ln -sf "$CURSOR_AGENT_DIR/cursor-agent" /usr/local/bin/agent; \
    CURSOR_AGENT_ACTUAL="$(cursor-agent --version)"; \
    echo "cursor-agent version: $CURSOR_AGENT_ACTUAL"; \
    case "$CURSOR_AGENT_ACTUAL" in \
      *"$CURSOR_AGENT_VERSION"*|*"${CURSOR_AGENT_VERSION%%-*}"*) ;; \
      *) echo "cursor-agent version mismatch: expected ${CURSOR_AGENT_VERSION}, got ${CURSOR_AGENT_ACTUAL}" >&2; exit 1 ;; \
    esac

COPY --from=build /out/any-ai-cli /usr/local/bin/any-ai-cli
COPY deploy/docker/entrypoint.sh /usr/local/bin/aac-entrypoint.sh
RUN chmod +x /usr/local/bin/aac-entrypoint.sh

# provider CLI の更新はイメージ再ビルドで行う（非 root 実行時の /usr/lib 書込を防ぐ）
ENV DISABLE_AUTOUPDATER=1

# ubuntu:24.04 同梱の一般ユーザー ubuntu (uid 1000) で実行。
# host 側 /srv/any-ai-cli/work/<user> は uid 1000 に chown して bind mount する。
# イメージ既定で sudo / adm グループに属しているため外す（sudo バイナリ自体も
# 未導入だが、多層防御として権限の芽を残さない）
RUN gpasswd -d ubuntu sudo && gpasswd -d ubuntu adm
USER ubuntu
ENV HOME=/home/ubuntu
WORKDIR /home/ubuntu

EXPOSE 48000
ENTRYPOINT ["/usr/local/bin/aac-entrypoint.sh"]
