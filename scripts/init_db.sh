#!/usr/bin/env bash
set -eo pipefail

# ========== 配置项（完全兼容原版环境变量覆盖） ==========
DB_USER=${POSTGRES_USER:=postgres}
DB_PASSWORD=${POSTGRES_PASSWORD:=password}
DB_NAME=${POSTGRES_DB:=newsletter}
DB_PORT=${POSTGRES_PORT:=5432}
CONTAINER_NAME="zero2prod-postgres"
POSTGRES_IMAGE="postgres:16"

# ========== 预检查：端口占用检测 ==========
if ss -tulpn 2>/dev/null | grep -q ":${DB_PORT}.*LISTEN"; then
  # 判断是不是Docker进程占用
  if ! ss -tulpn 2>/dev/null | grep ":${DB_PORT}.*LISTEN" | grep -q "docker"; then
    echo "❌ 端口 ${DB_PORT} 被系统进程占用（通常是WSL自带的PostgreSQL服务）"
    echo "执行以下命令关闭后重试："
    echo "  sudo systemctl stop postgresql"
    echo "  sudo systemctl disable postgresql"
    exit 1
  fi
fi

# ========== 智能容器管理（不复用不重复创建） ==========
if docker ps -a --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
  if docker ps --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
    echo "✅ Postgres 容器已在运行，跳过启动"
  else
    echo "🔄 发现已停止的容器，正在启动..."
    docker start ${CONTAINER_NAME}
  fi
else
  echo "🚀 新建并启动 Postgres 容器..."
  docker run \
    --name ${CONTAINER_NAME} \
    -e POSTGRES_USER=${DB_USER} \
    -e POSTGRES_PASSWORD=${DB_PASSWORD} \
    -e POSTGRES_DB=${DB_NAME} \
    -p "${DB_PORT}":5432 \
    -d \
    --restart unless-stopped \
    ${POSTGRES_IMAGE} \
    postgres -N 1000
fi

# ========== 等待数据库就绪（不依赖宿主机psql） ==========
echo "⏳ 等待数据库启动就绪..."
until docker exec ${CONTAINER_NAME} pg_isready -U ${DB_USER} -d ${DB_NAME} -q; do
  sleep 1
done
echo "✅ Postgres 启动完成，端口：${DB_PORT}"

# ========== 自动建库 + 执行迁移 ==========
export DATABASE_URL="postgres://${DB_USER}:${DB_PASSWORD}@localhost:${DB_PORT}/${DB_NAME}"

# 检查sqlx-cli是否安装
if ! command -v sqlx &> /dev/null; then
  echo "⚠️  未检测到 sqlx-cli，已跳过自动迁移"
  echo "安装命令：cargo install --version=0.6.0 sqlx-cli --no-default-features --features postgres,native-tls"
  echo "数据库连接串：${DATABASE_URL}"
  exit 0
fi

# 仅当数据库不存在时才创建，避免重复执行报错
if ! sqlx database check >/dev/null 2>&1; then
  echo "📦 创建数据库 ${DB_NAME}"
  sqlx database create
fi


echo "🔄 执行数据库迁移..."
sqlx migrate run

echo ""
echo "🎉 初始化完成！数据库连接串："
echo "${DATABASE_URL}"