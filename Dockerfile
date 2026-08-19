# 获取 cargo chef
FROM lukemathwalker/cargo-chef:latest-rust-1.97.1 AS chef
WORKDIR /app
# 兼容 Debian 11/12：替换所有 apt 源文件中的域名
RUN sed -i 's/deb.debian.org/mirrors.tuna.tsinghua.edu.cn/g; s/security.debian.org/mirrors.tuna.tsinghua.edu.cn/g' /etc/apt/sources.list 2>/dev/null || true && \
    find /etc/apt/sources.list.d -type f \( -name "*.list" -o -name "*.sources" \) -exec sed -i 's/deb.debian.org/mirrors.tuna.tsinghua.edu.cn/g; s/security.debian.org/mirrors.tuna.tsinghua.edu.cn/g' {} + 2>/dev/null || true && \
    apt-get update && apt-get install -y --no-install-recommends lld clang && \
    apt-get clean && rm -rf /var/lib/apt/lists/*

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# 构建阶段
FROM chef AS builder
RUN mkdir -p /usr/local/cargo && \
    echo '[source.crates-io]' > /usr/local/cargo/config.toml && \
    echo 'replace-with = "ustc"' >> /usr/local/cargo/config.toml && \
    echo '[source.ustc]' >> /usr/local/cargo/config.toml && \
    echo 'registry = "sparse+https://mirrors.ustc.edu.cn/crates.io-index/"' >> /usr/local/cargo/config.toml
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
ENV SQLX_OFFLINE=true
RUN cargo build --release

# 运行时阶段
FROM debian:bullseye-slim AS runtime
WORKDIR /app
RUN sed -i 's/deb.debian.org/mirrors.tuna.tsinghua.edu.cn/g' /etc/apt/sources.list && \
    sed -i 's/security.debian.org/mirrors.tuna.tsinghua.edu.cn/g' /etc/apt/sources.list && \
    apt-get update && \
    apt-get install -y --no-install-recommends openssl ca-certificates && \
    apt-get clean && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/zero2prod zero2prod
COPY configuration configuration
ENV APP_ENVIRONMENT=production
ENTRYPOINT ["./zero2prod"]