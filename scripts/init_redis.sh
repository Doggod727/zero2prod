#!/usr/bin/env bash
set -euo pipefail

# 如果已有名为 redis 的容器在运行，提示并退出
RUNNING_CONTAINER=$(docker ps --filter 'name=redis' --format '{{.ID}}')
if [[ -n "$RUNNING_CONTAINER" ]]; then
  echo >&2 "There is a redis container already running, kill it with:"
  echo >&2 "  docker kill $RUNNING_CONTAINER"
  exit 1
fi

# 启动 Redis 容器
docker run \
  --name "redis_$(date '+%s')" \
  -p "6379:6379" \
  -d \
  redis:6

echo >&2 "Redis is ready to go"