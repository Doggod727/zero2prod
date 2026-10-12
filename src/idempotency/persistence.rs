//! src/idempotency/persistence.rs

use super::IdempotencyKey;
use actix_web::body::to_bytes;
use actix_web::HttpResponse;
use reqwest::StatusCode;
use sqlx::postgres::{PgHasArrayType, PgTypeInfo};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

// 三种行的清扫规则不同，这是本模块的核心：
//   已完成            → 缓存，超 TTL 删除
//   处理中且新鲜      → 有请求在跑，谁也不能动
//   处理中且很旧      → 请求死了，让下一个请求【接管】（租约语义，不是 TTL）
//
// ⚠️ 删"处理中"的行会同时造成：原请求 save_response 影响 0 行、成果静默丢失；
//    用户重试撞上"INSERT 冲突但查不到已存响应" → 500。两条都实测过。

/// 已完成条目保留多久。
///
/// ⚠️ 必须【大于最长处理时间】：设小了，一次慢请求还没走到 save_response
///    键就被接管 → 同一个 key 两份在跑 → 重复发信。
///
/// ⚠️ 必须是整数：SQL 用 make_interval(mins => $n)，而它的 mins/hours 只收 integer。
///    定义成 f64 的话 sqlx 会把参数绑成 float8，Postgres 报
///    "function make_interval(mins => double precision) does not exist"，
///    写成 $n::int 也救不了（协议层已定成 float8）。
pub const COMPLETED_TTL_MINUTES: i32 = 24 * 60;

/// 处理中条目超过多久算"请求已经死了"。
///
/// 这管的是崩溃恢复，不是缓存过期：客户端断连 / 进程被杀会留下一行永远是 NULL
/// 的记录，没有这个逃生门，那个幂等键就被永久锁死（用户重试多少次都是 500）。
/// 取 5 分钟：比正常处理时间长一个量级，又远短于 COMPLETED_TTL_MINUTES。
pub const STALE_PROCESSING_TTL_MINUTES: i32 = 5;

/// sweeper 间隔。不影响正确性 —— 过期靠 try_processing 的惰性接管保证，
/// sweeper 只回收磁盘，所以可以取大。
pub const SWEEP_INTERVAL_SECONDS: u64 = 900;

const STALE_PROCESSING_TTL_MESSAGE: &str =
    "A request with the same idempotency key is still being processed";

#[derive(Debug, sqlx::Type)]
#[sqlx(type_name = "header_pair")]
struct HeaderPairRecord {
    name: String,
    value: Vec<u8>,
}

impl PgHasArrayType for HeaderPairRecord {
    fn array_type_info() -> PgTypeInfo {
        PgTypeInfo::with_name("_header_pair")
    }
}

pub async fn get_saved_response(
    pool: &PgPool,
    idempotency_key: &IdempotencyKey,
    user_id: Uuid,
) -> Result<Option<HttpResponse>, anyhow::Error> {
    let saved_response = sqlx::query!(
        r#"
        SELECT
            response_status_code as "response_status_code!",
            response_headers as "response_headers!: Vec<HeaderPairRecord>",
            response_body as "response_body!"
        FROM idempotency
        WHERE
            user_id = $1 AND
            idempotency_key = $2
        "#,
        user_id,
        idempotency_key.as_ref()
    )
    .fetch_optional(pool)
    .await?;

    if let Some(r) = saved_response {
        let status_code = StatusCode::from_u16(r.response_status_code.try_into()?)?;
        let mut response = HttpResponse::build(status_code);
        for HeaderPairRecord { name, value } in r.response_headers {
            response.append_header((name, value));
        }
        Ok(Some(response.body(r.response_body)))
    } else {
        Ok(None)
    }
}

pub async fn save_response(
    mut transaction: Transaction<'static, Postgres>,
    idempotency_key: &IdempotencyKey,
    user_id: Uuid,
    http_response: HttpResponse,
) -> Result<HttpResponse, anyhow::Error> {
    let (response_head, body) = http_response.into_parts();
    let body = to_bytes(body).await.map_err(|e| anyhow::anyhow!("{}", e))?;
    let status_code = response_head.status().as_u16() as i16;
    let headers = {
        let mut h = Vec::with_capacity(response_head.headers().len());
        for (name, value) in response_head.headers().iter() {
            let name = name.as_str().to_owned();
            let value = value.as_bytes().to_owned();
            h.push(HeaderPairRecord { name, value });
        }
        h
    };

    // query_unchecked!：header_pair[] 这个复合类型 sqlx 推断不出可空性，校验器会拒绝。
    // 代价是这 5 个参数的类型不在编译期核对。
    let updated = sqlx::query_unchecked!(
        r#"
        UPDATE idempotency
        SET
            response_status_code = $3,
            response_headers = $4,
            response_body = $5
        WHERE
            user_id = $1 AND
            idempotency_key = $2
        "#,
        user_id,
        idempotency_key.as_ref(),
        status_code,
        headers,
        body.as_ref()
    )
    .execute(&mut transaction)
    .await?
    .rows_affected();

    // ⚠️ 0 行就报错。不查的话，这一行若消失，UPDATE 静默影响 0 行然后照常 commit ——
    //    表现是"发布成功但幂等键里没存响应"，用户重试会被当成全新请求，
    //    于是发出第二期 newsletter。宁可炸成 500，也不要静默失去幂等性。
    if updated == 0 {
        anyhow::bail!(
            "The idempotency record for key {} disappeared before its response could be saved",
            idempotency_key.as_ref()
        );
    }

    transaction.commit().await?;

    let http_response = response_head.set_body(body).map_into_boxed_body();
    Ok(http_response)
}

// 实测 NextAction = 928B，其中 StartProcessing 928B、ReturnSavedResponse 104B ——
// 大的是 sqlx 的 Transaction 本身。Box 掉只省每请求 824 字节栈空间，
// 而它每请求只构造一次就立刻被 match 消费，代价是多一次堆分配。接受。
#[allow(clippy::large_enum_variant)]
pub enum NextAction {
    StartProcessing(Transaction<'static, Postgres>),
    ReturnSavedResponse(HttpResponse),
}

pub async fn try_processing(
    pool: &PgPool,
    idempotency_key: &IdempotencyKey,
    user_id: Uuid,
) -> Result<NextAction, anyhow::Error> {
    let mut transaction = pool.begin().await?;
    let n_inserted_rows = sqlx::query!(
        r#"
        INSERT INTO idempotency(user_id, idempotency_key, create_at)
        VALUES ($1, $2, now())
        ON CONFLICT DO NOTHING
        "#,
        user_id,
        idempotency_key.as_ref()
    )
    .execute(&mut transaction)
    .await?
    .rows_affected();

    if n_inserted_rows > 0 {
        return Ok(NextAction::StartProcessing(transaction));
    }

    // 冲突 → 行已存在。靠两个字段区分三种情况：
    //   response_body 为 NULL  → 处理中；create_at 够旧 → 那个请求已经死了
    let row = sqlx::query!(
        r#"
        SELECT
            -- ⚠️ `!` 是 sqlx 的显式非空标注，不是笔误：不加它，可空列上的
            --    IS NULL 会被推断成 Option<bool>，Rust 侧 `!r.processing` 编译不过。
            --    加 COALESCE 也没用（0.6 的推断不看函数语义）。
            response_body IS NULL AS "processing!",
            -- 「还活着吗」这个判断直接交给数据库算，不在 Rust 侧再写一遍。
            -- 理由：租约时长（$3）是同一个数，如果 Rust 用 chrono 算、SQL 用
            -- make_interval 算，两处迟早会漂移；而且 now() 必须来自数据库，
            -- 应用服务器和数据库的时钟不一定一致，拿应用时间比库时间会误判。
            create_at > now() - make_interval(mins => $3) AS "still_alive!"
        FROM idempotency
        WHERE user_id = $1 AND idempotency_key = $2
        "#,
        user_id,
        idempotency_key.as_ref(),
        STALE_PROCESSING_TTL_MINUTES
    )
    .fetch_optional(&mut transaction)
    .await?;

    match row {
        // 有已存响应 → 重放。这是幂等键最核心的那条路径：
        // 用户按了两次、网络重试、F5 —— 都不该产生第二期 newsletter。
        Some(r) if !r.processing => {
            let saved_response = get_saved_response(pool, idempotency_key, user_id)
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!("We expected a saved response, we didn't find it")
                })?;
            Ok(NextAction::ReturnSavedResponse(saved_response))
        }
        // 处理中且不够旧 → 真的有请求在跑，不能接管。
        Some(r) if r.still_alive => {
            anyhow::bail!(STALE_PROCESSING_TTL_MESSAGE)
        }
        // 处理中但已经够旧 → 那个请求死了（断连 / 被杀 / 部署中断）。
        // 【接管】而不是删除：把 create_at 推到 now()，这一行就此归属本次请求。
        //
        // 一个语句完成"检查 + 抢占"，中间没有缝：
        // 如果先 SELECT 判断再 UPDATE，两个并发请求会同时判定"它死了"、
        // 同时接管 → 同一个 key 两份在跑 → 重复发信。
        // 这条 UPDATE 的 WHERE 里带着 create_at 条件，只有一个能改到行。
        Some(_) => {
            let taken = sqlx::query!(
                r#"
                UPDATE idempotency
                SET create_at = now()
                WHERE user_id = $1
                  AND idempotency_key = $2
                  AND response_body IS NULL
                  AND create_at < now() - make_interval(mins => $3)
                "#,
                user_id,
                idempotency_key.as_ref(),
                STALE_PROCESSING_TTL_MINUTES
            )
            .execute(&mut transaction)
            .await?
            .rows_affected();

            if taken > 0 {
                tracing::warn!(
                    user_id = %user_id,
                    idempotency_key = idempotency_key.as_ref(),
                    "Took over an expired in-flight idempotency key; \
                     the original request most likely died mid-flight"
                );
                return Ok(NextAction::StartProcessing(transaction));
            }
            // 0 行：在这一瞬间别人先接管了，或者原来那个请求刚好写完了响应。
            // 两种情况的正确处理都是"别抢"。
            anyhow::bail!(STALE_PROCESSING_TTL_MESSAGE)
        }
        // 行没了：唯一的可能是它在这一瞬间被离线的归档/清理删掉了。
        None => anyhow::bail!(STALE_PROCESSING_TTL_MESSAGE),
    }
}

// ============================================================
// 清扫（sweeper）
// ============================================================

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SweepOutcome {
    /// 删除的已完成条目数。
    pub deleted_completed: u64,
    /// 判定为"请求已死"、把 create_at 推到现在的条目数。
    pub recycled_stale: u64,
}

/// 清扫一轮。返回这一轮各做了多少，便于测试断言与打点。
///
/// ⚠️ 这里【没有】DELETE 掉处理中的行，这是刻意的：
///    删掉"处理中"的行会让那个还在跑的请求 save_response 时 UPDATE 0 行
///    （成果静默消失），并且用户重试同一个 key 时会拿到 500。
///    两条都在 README 的决策卡里有实测记录。
///    处理中的行只能靠 try_processing 的惰性接管变成"活跃"，
///    或者等它自己写完响应、变成"已完成"，再被这里按 TTL 删掉。
pub async fn sweep(pool: &PgPool) -> Result<SweepOutcome, sqlx::Error> {
    let deleted = sqlx::query!(
        r#"
        DELETE FROM idempotency
        WHERE response_body IS NOT NULL
          AND create_at < now() - make_interval(mins => $1)
        "#,
        COMPLETED_TTL_MINUTES
    )
    .execute(pool)
    .await?
    .rows_affected();

    // 处理中且超过租约的行：把 create_at 推到 now()，让它重新变成"刚被领取"。
    //
    // 为什么 sweeper 也要做这件事，而不完全交给 try_processing 的惰性接管：
    //   如果等了 5 分钟的那个用户再也没回来重试，这一行就永远是 NULL + 永远旧，
    //   谁也扫不掉（因为按上面的规则不许删处理中的行）→ 这一行永久占着空间。
    //   把它推新之后，它至少还能在下一次进来时被正常重放/接管。
    //   注意这【不】解决泄漏本身：真正的兜底是"归档"而不是"删除"，
    //   留待以后做（见 README 的待办）。
    let recycled = sqlx::query!(
        r#"
        UPDATE idempotency
        SET create_at = now()
        WHERE response_body IS NULL
          AND create_at < now() - make_interval(mins => $1)
        "#,
        STALE_PROCESSING_TTL_MINUTES
    )
    .execute(pool)
    .await?
    .rows_affected();

    Ok(SweepOutcome {
        deleted_completed: deleted,
        recycled_stale: recycled,
    })
}

/// 周期性清扫，直到 shutdown 被置位。
pub async fn run_sweeper_until_stopped(
    pool: PgPool,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<(), anyhow::Error> {
    loop {
        // 和 worker_loop 一致：先查一次标志，再干活。
        // 这样收到信号后【不会】再开启新一轮清扫。
        if *shutdown.borrow() {
            tracing::info!("Sweeper received shutdown signal, stopping.");
            return Ok(());
        }

        match sweep(&pool).await {
            Ok(outcome) if outcome != SweepOutcome::default() => {
                tracing::info!(
                    deleted_completed = outcome.deleted_completed,
                    recycled_stale = outcome.recycled_stale,
                    "Idempotency sweep finished"
                );
            }
            Ok(_) => {}
            // 清扫失败【不影响任何请求】：它只是回收空间，过期语义由
            // try_processing 的惰性接管保证。所以记一条日志然后继续，
            // 不要 return Err —— 那会把整个任务结束掉，之后再也不清扫了。
            Err(e) => {
                tracing::error!(
                    error.cause_chain = ?e,
                    error.message = %e,
                    "Idempotency sweep failed; will retry on the next tick"
                );
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(SWEEP_INTERVAL_SECONDS)) => {}
            _ = shutdown.changed() => {}
        }
    }
}
