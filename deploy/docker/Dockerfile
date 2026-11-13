# syntax=docker/dockerfile:1
# any-ai-cli マルチユーザー運用イメージ（1 ユーザー = 1 コンテナ）
#
# build context はリポジトリルート。
# 通常は GitHub Actions（.github/workflows/docker-image.yml）がビルドして
# ghcr.io/ishizakahiroshi/many-ai-cli へ publish し、リモートサーバーは pull するだけ
# （users/<user>.yaml の image: 参照と aac-update.sh の cron が更新を担う）。
# 開発中バイパス（リモートサーバーローカルビルド）の例:
#   docker build -t ghcr.io/ishizakahiroshi/many-ai-cli:dev \
#     -f src/deploy/docker/Dockerfile src
#
# 設計の前提（docs/local/archive/plan_docker-multiuser-isolation.md）:
# - Hub はコンテナ内 loopback の $HUB_PORT で listen する（Host/Origin 検証が
#   ポート完全一致を要求するため、ユーザー割当ポートと一致させる）
# - 外向きの受け口は socat（0.0.0.0:48000 → 127.0.0.1:$HUB_PORT）で、
#   host 側 publish は 127.0.0.1 限定。外部公開はしない

# --- pinned toolchain stages ---
# These are Docker Hub manifest-list digests retrieved from the official image
# repositories on 2026-09-05. Tags remain visible for operator readability.
FROM oven/bun:1.4.4@sha256:1d22548f177f4ff38e0c15baa3c2c46c6c2060ba2d1bc573bb4720b407548f1d AS bun-runtime
FROM node:22-bookworm@sha256:8a34c4ab3ea2c5cd194f07e317b2a8f09461d3c8b05c4e34c8ccd56d56024c4d AS node-runtime

# --- web build stage: TypeScript フロントを web/dist に生成 ---
# bun は依存取得のみに使い、ビルドスクリプト実行は node のまま（攻撃面を最小化）
FROM node-runtime AS web-build
COPY --from=bun-runtime /usr/local/bin/bun /usr/local/bin/bun
WORKDIR /src/web
COPY web/package.json web/bun.lock web/bunfig.toml ./
RUN bun install --frozen-lockfile
COPY web ./
RUN bun run build

# --- provider CLI stage: exact versions from the tracked npm lockfile ---
FROM node-runtime AS provider-cli-build
WORKDIR /opt/provider-cli
ARG PROVIDER_CLAUDE_CODE_VERSION=2.1.179
ARG PROVIDER_CODEX_VERSION=0.137.0
ARG PROVIDER_COPILOT_VERSION=1.0.59
COPY deploy/docker/provider-cli/package.json deploy/docker/provider-cli/package-lock.json ./
RUN npm ci --omit=dev

# --- build stage: any-ai-cli バイナリ（CGO 不要・web/dist は go:embed 同梱）---
FROM golang:1.25-bookworm@sha256:3b4a11519ad929d1e1d261a12cff056f0c85b735253d7d861346b9c6f8b36437 AS build
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY . .
COPY --from=web-build /src/web/dist ./web/dist
RUN CGO_ENABLED=0 go build -trimpath -ldflags "-s -w" -o /out/many-ai-cli ./cmd/many-ai-cli

# --- runtime stage: Ubuntu 24.04 + Node.js 22 + provider CLI ---
FROM ubuntu:24.04@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517
ARG CURSOR_AGENT_VERSION=2026.06.03-0bbb28e
ARG CURSOR_AGENT_X64_SHA256=f4250e91feea78bb55e1c48b5fc830a57d3dd3681b04652f629726c5b9c8f24a
ARG CURSOR_AGENT_ARM64_SHA256=b207208ee96c9230fc19baf1457f25206430ef3b63d750f690afc20885ba812f
ARG TARGETARCH
ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8

# git: Hub の Git 機能・provider CLI が実行時に必須
# socat: Hub(loopback) への中継。curl/ca-certificates: GitHub からの定義取得
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
       ca-certificates curl git socat openssh-client procps less nano jq \
  && apt-get clean && rm -rf /var/lib/apt/lists/*
COPY --from=provider-cli-build /usr/local /usr/local
COPY --from=provider-cli-build /opt/provider-cli /opt/provider-cli
ENV PATH="/opt/provider-cli/node_modules/.bin:${PATH}"
# provider CLI は package.json/package-lock.json と npm ci で固定する。
# 更新は exact version、lockfile、Docker input pin、checker、smoke を同じ変更で行う。

# cursor-agent: 公式インストーラは「最新」固定のため、インストーラが参照する
# versioned package URL を直接取得して pin する。/home/ubuntu は実行時に volume で
# 覆われるため、全ユーザー共通の /opt へ配置する。
RUN set -eu; \
    case "${TARGETARCH:-amd64}" in \
      amd64) CURSOR_AGENT_ARCH="x64"; EXPECTED_SHA256="${CURSOR_AGENT_X64_SHA256}" ;; \
      arm64) CURSOR_AGENT_ARCH="arm64"; EXPECTED_SHA256="${CURSOR_AGENT_ARM64_SHA256}" ;; \
      *) echo "unsupported TARGETARCH for cursor-agent: ${TARGETARCH:-unset}" >&2; exit 1 ;; \
    esac; \
    case "$EXPECTED_SHA256" in *[!0-9a-f]*|'') echo "invalid cursor-agent sha256" >&2; exit 1 ;; esac; \
    [ "${#EXPECTED_SHA256}" -eq 64 ] || { echo "invalid cursor-agent sha256 length" >&2; exit 1; }; \
    CURSOR_AGENT_DIR="/opt/cursor-agent/versions/${CURSOR_AGENT_VERSION}"; \
    mkdir -p "$CURSOR_AGENT_DIR"; \
    curl -fSL --retry 3 -o /tmp/cursor-agent.tar.gz "https://downloads.cursor.com/lab/${CURSOR_AGENT_VERSION}/linux/${CURSOR_AGENT_ARCH}/agent-cli-package.tar.gz"; \
    echo "$EXPECTED_SHA256  /tmp/cursor-agent.tar.gz" | sha256sum -c -; \
    tar --strip-components=1 -xzf /tmp/cursor-agent.tar.gz -C "$CURSOR_AGENT_DIR"; \
    rm -f /tmp/cursor-agent.tar.gz; \
    ln -sf "$CURSOR_AGENT_DIR/cursor-agent" /usr/local/bin/cursor-agent; \
    ln -sf "$CURSOR_AGENT_DIR/cursor-agent" /usr/local/bin/agent; \
    CURSOR_AGENT_ACTUAL="$(cursor-agent --version)"; \
    echo "cursor-agent version: $CURSOR_AGENT_ACTUAL"; \
    case "$CURSOR_AGENT_ACTUAL" in \
      *"$CURSOR_AGENT_VERSION"*|*"${CURSOR_AGENT_VERSION%%-*}"*) ;; \
      *) echo "cursor-agent version mismatch: expected ${CURSOR_AGENT_VERSION}, got ${CURSOR_AGENT_ACTUAL}" >&2; exit 1 ;; \
    esac

COPY --from=build /out/many-ai-cli /usr/local/bin/many-ai-cli
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
