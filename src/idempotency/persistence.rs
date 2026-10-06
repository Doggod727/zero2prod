//! src/idempotency/persistence.rs

use super::IdempotencyKey;
use actix_web::body::to_bytes;
use actix_web::HttpResponse;
use reqwest::StatusCode;
use sqlx::postgres::{PgHasArrayType, PgTypeInfo};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

// ============================================================
// 过期策略：三个时间量，各自管不同的事
// ============================================================
//
// 这张表有三个状态，清扫它们的合法性完全不同 —— 这是本模块最核心的一条：
//
//   已完成（response_body IS NOT NULL）  →  它就是一份缓存，过期即失去重放能力。
//                                          这正是 TTL 的语义，超时【删除】。
//   处理中（response_body IS NULL，新）  →  有请求正在跑。删它会发生两件坏事：
//                                            ① 原请求回来 save_response 时 UPDATE 0 行，
//                                               它的成果被静默丢弃；
//                                            ② 用户重试同一个 key 时会撞上
//                                               "INSERT 冲突但查不到已存响应" → 500。
//                                          （两条都实测过，见 README 的决策卡。）
//   处理中 + 很旧（请求崩了）             →  不能删，要让下一个请求【接管】。
//                                          这是租约（lease）语义，不是 TTL 语义，
//                                          对应 Redis 那边的看门狗/续租，
//                                          而不是 maxmemory 淘汰。

/// 已完成条目保留多久。
///
/// ⚠️ 必须【大于最长处理时间】。设小了会这样：一次慢请求（比如给几千人发信）
///    还没走到 save_response，键就过期被下一个请求接管 → 同一个 key 有两份在跑
///    → 重复发信。幂等键的全部价值就在这条线上，所以宁可给足余量。
///
/// 单位是分钟、类型是整数：SQL 那边用 make_interval(mins => $n)，
/// 而 make_interval 的 mins/hours 是 **integer** 参数。
/// 如果这里定义成 f64，sqlx 会按 Rust 值把参数绑成 float8，
/// Postgres 就会报：
///     function make_interval(mins => double precision) does not exist
/// 而且即使写成 $n::int 也没用 —— 参数类型已经在协议层定成 float8 了。
/// 结论：时间量要么用整数常量，要么用 `$n * interval '1 minute'` 这种乘式，
/// 不要指望在函数参数位置做类型转换。
pub const COMPLETED_TTL_MINUTES: i32 = 24 * 60;

/// 处理中条目超过多久算"请求已经死了"。
///
/// 这个值管的是【崩溃恢复】，不是【缓存过期】：
/// 客户端提交后断连、worker 被杀、部署中断 —— 那一行 response_body 会永远是 NULL。
/// 没有这个逃生门的话，这个幂等键就被【永久锁死】：用户重试多少次都是 500。
///
/// 取 5 分钟的依据：比"正常处理时间"长一个量级（本项目一次发布是秒级），
/// 又远比 COMPLETED_TTL_MINUTES 短，所以被误接管的窗口极小；
/// 而真被误接管也只是多跑一次（幂等键本来就是为这种情况准备的兜底）。
pub const STALE_PROCESSING_TTL_MINUTES: i32 = 5;

/// sweeper 两轮之间的间隔。
///
/// 它【不影响正确性】—— 过期是靠 try_processing 里的惰性接管保证的，
/// sweeper 只负责回收磁盘。所以这个值可以取大：一轮就是一个 DELETE，
/// 跑得勤没有收益，只是白占连接。
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

    // query_unchecked! 而不是 query!：header_pair[] 这个复合类型 sqlx 推断不出可空性，
    // 校验器会拒绝。这里用 unchecked 是既有选择，代价是这 5 个参数的类型不在编译期核对。
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

    // ⚠️ 检查 rows_affected，0 行直接报错。
    //
    // 不加这个检查的话，一旦这一行在事务期间消失（被运维手工删、或将来加了
    // 归档任务），UPDATE 会静默影响 0 行、【然后照常 commit】——
    // 表现是"发布成功了，但幂等键里没存响应"：用户重试同一个 key 时会被
    // 当成全新请求，于是发出【第二期】newsletter。
    // 这是幂等机制最严重的一种失效，而且完全没有报错。
    // 宁可让它在这里炸成 500，也不要静默地失去幂等性。
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

// Clippy 会问：为什么一个 enum 的两个 variant 差了 8 倍？
// 实测数据：NextAction = 928B，StartProcessing = 928B，ReturnSavedResponse = 104B。
// 大的那个是 sqlx 的 Transaction 本身（928B），不是我们塞进去的东西 ——
// 它是「一个已开始的、握着一个池连接的事务」，体积由 sqlx 决定。
//
// 为什么不 Box 掉它：省下的是每个请求的 824 字节栈空间，
// 而这个 enum 每个请求只构造一次、立刻就被 match 消费掉，
// 代价是每次幂等请求多一次堆分配、代码里多一层解引用。
// 收益为零、成本非零，所以这里明确接受。
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

    // 冲突了 → 这一行已经存在。它可能是三种东西之一，靠两个字段判断：
    //   response_body 是否为 NULL   →  是"已完成"还是"处理中"
    //   create_at 是否够旧          →  处理中那个请求还活着吗
    let row = sqlx::query!(
        r#"
        SELECT
            -- ⚠️ `as "processing!"` 这个感叹号是 sqlx 的【显式非空标注】，不是笔误。
            --    不加它：sqlx 看到可空列上的 IS NULL / 比较表达式，会把结果推断成
            --    Option<bool>，于是 Rust 侧 `!r.processing` 直接编译不过
            --    （cannot apply unary operator `!` to type `Option<bool>`）。
            --    加 COALESCE 也没用 —— 0.6 的推断不看函数语义。
            --    这类"数据库说它非空、但类型系统不知道"的场合，就是 `!` 的用途。
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
