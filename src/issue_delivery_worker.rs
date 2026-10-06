//! src/issue_delivery_worker.rs

use crate::configurations::Settings;
use crate::domain::SubscriberEmail;
use crate::email_client::EmailClient;
use crate::startup::get_connection_pool;
use sqlx::{PgPool};
use std::time::Duration;
use rand::Rng;
use uuid::Uuid;

/// 租约期：领取任务时把 next_retry_at 推到"现在 + 租约"，
/// 在这段时间内别的 worker 领不到它。
///
/// **必须大于 email_client 的 timeout（配置里是 10 秒）**，否则会出现
/// "发信还没超时、租约先过期" —— 两个 worker 同时在发同一封邮件。
const LEASE_SECONDS: f64 = 60.0;

/// 退避基数（秒）：第 0 次失败后最坏等这么久。
const RETRY_BASE_SECONDS: u64 = 5;

/// 退避上限（秒）：指数增长到这里就封顶，避免退避到天文数字。
const MAX_BACKOFF_SECONDS: u64 = 3600;

/// 最大领取次数。`attempts` 在 dequeue_task 里就 +1 了，
/// 所以到达它时说明这条任务已经被真正尝试发送 MAX_ATTEMPTS - 1 次，应当判死。
const MAX_ATTEMPTS: i32 = 5;

pub enum ExecutionOutcome {
    TaskCompleted,
    EmptyQueue,
}

struct NewsletterIssue {
    title: String,
    text_content: String,
    html_content: String,
}

pub enum DeliveryTask {
    Newsletter{issue_id: Uuid, recipient: SubscriberEmail},
    Confirmation{token: String, recipient: SubscriberEmail}
}
pub struct Task {
    id: Uuid,
    /// 这条任务已经被领取过几次（含本次）。失败时用它算退避。
    attempts: i32,
    kind: DeliveryTask,
}
#[tracing::instrument(skip_all)]
async fn get_issue(pool: &PgPool, issue_id: Uuid) -> Result<NewsletterIssue, anyhow::Error> {
    let issue = sqlx::query_as!(
        NewsletterIssue,
        r#"
        SELECT title, text_content, html_content
        FROM newsletter_issues
        WHERE 
            newsletter_issue_id = $1
        "#,
        issue_id
    )
    .fetch_one(pool)
    .await?;
    Ok(issue)
}

#[tracing::instrument(
    skip_all,
    fields(
        newsletter_issue_id = tracing::field::Empty,
        subscriber_email = tracing::field::Empty
    ),
    err
)]
pub async fn try_execute_task(
    pool: &PgPool,
    email_client: &EmailClient,
    base_url: &str
) -> Result<ExecutionOutcome, anyhow::Error> {
    let task = dequeue_task(pool).await?;
    let Some(task) = task else {
        return Ok(ExecutionOutcome::EmptyQueue);
    };
    let Task { id, attempts, kind } = task;
    let (recipient, subject, html_content, text_content) = match kind {
        DeliveryTask::Newsletter { issue_id, recipient } => {
            let newsletter_issue = get_issue(pool, issue_id).await?;
            (recipient, newsletter_issue.title, newsletter_issue.html_content, newsletter_issue.text_content)
        }
        DeliveryTask::Confirmation { token, recipient } => {
            let link = format!(
                "{}/subscriptions/confirm?subscription_token={}",
                base_url, token
            );
            let text = format!("Visit {link} to confirm your subscription.");
            let html = format!("Click <a href=\"{link}\">here</a> to confirm your subscription.");
            (recipient, "Welcome!".to_string(), html, text)
        }
    };
    match email_client.send_email(&recipient, &subject, &html_content, &text_content).await {
        Ok(()) => delete_task(pool, id).await?,
        // attempts 也要传过去 —— 退避时长依赖它
        Err(e) => record_failure(pool, id, attempts, &e.to_string()).await?
    };
    Ok(ExecutionOutcome::TaskCompleted)
}

#[tracing::instrument(skip_all)]
async fn dequeue_task(
    pool: &PgPool,
) -> Result<Option<Task>, anyhow::Error> {
    loop {
        let claimed = {
            let mut transaction = pool.begin().await?;
            let r = sqlx::query!(
                r#"
                UPDATE email_delivery_queue
                SET next_retry_at = now() + make_interval(secs => $1),
                    attempts = attempts + 1
                WHERE delivery_id = (
                    SELECT delivery_id
                    FROM email_delivery_queue e
                    WHERE next_retry_at <= now()
                    FOR UPDATE
                    SKIP LOCKED
                    LIMIT 1
                )
                RETURNING delivery_id, task_type, recipient, newsletter_issue_id,
                          subscription_token, attempts, last_error
                "#,
                LEASE_SECONDS as f64
            )
                .fetch_optional(&mut transaction)
                .await?;
            transaction.commit().await?;
            r
        };

        let Some(record) = claimed else {
            return Ok(None)
        };
        // attempts 在领取时已经 +1，所以到达 MAX_ATTEMPTS 时说明
        // 这条任务已经被尝试过 MAX_ATTEMPTS - 1 次，不再重试，直接判死。
        if record.attempts >= MAX_ATTEMPTS {
            dead_letter_task(
                pool,
                record.delivery_id,
                &record.task_type,
                record.newsletter_issue_id,
                record.subscription_token.as_deref(),
                &record.recipient,
                // 优先用队列行里记着的最后一次真实失败原因；
                // 只有在从未记录过失败（last_error 为空）时，才退回一句通用说明。
                &record.last_error.clone().unwrap_or_else(|| {
                    format!("exhausted {} delivery attempts", MAX_ATTEMPTS)
                }),
                record.attempts,          // ← 你已经在 RETURNING 里取了，正好用上
            )
                .await?;
            delete_task(pool, record.delivery_id).await?;
            continue;
        }
        let recipient = match SubscriberEmail::parse(record.recipient.clone()) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(
                    delivery_id = %record.delivery_id,
                    recipient = %record.recipient,
                    "Invalid recipient, moving task to the dead letter queue: {e}"
                );
                dead_letter_task(
                    pool,
                    record.delivery_id,
                    &record.task_type,
                    record.newsletter_issue_id,
                    record.subscription_token.as_deref(),
                    &record.recipient,
                    &e,
                    record.attempts,          // ← 你已经在 RETURNING 里取了，正好用上
                )
                    .await?;
                delete_task(pool, record.delivery_id).await?;
                continue;
            }
        };
        let kind = match record.task_type.as_str() {
            "newsletter" => DeliveryTask::Newsletter {
                issue_id: record.newsletter_issue_id.ok_or_else(|| {
                    anyhow::anyhow!(
                        "newsletter task {} has no newsletter issue id",
                        record.delivery_id
                    )
                })?,
                recipient
            },
            "confirmation" => DeliveryTask::Confirmation {
                token: record.subscription_token.ok_or_else(|| {
                    anyhow::anyhow!(
                        "subscription confirm task {} has no subscription token",
                        record.delivery_id
                    )
                })?,
                recipient
            },
            // 未知类型是"永久失败"：不能返回 Err，否则 worker 会一直领到同一条
            // 并在 1 秒重试里空转。和重试超限一样直接进死信。
            other => {
                tracing::error!(
                    delivery_id = %record.delivery_id,
                    task_type = other,
                    "Unknown task_type, moving task to the dead letter queue"
                );
                dead_letter_task(
                    pool,
                    record.delivery_id,
                    &record.task_type,
                    record.newsletter_issue_id,
                    record.subscription_token.as_deref(),
                    &record.recipient,
                    &format!("unknown task_type: {other}"),
                    record.attempts,
                )
                .await?;
                delete_task(pool, record.delivery_id).await?;
                continue;
            }
        };
        return Ok(Some(Task {
            id: record.delivery_id,
            attempts: record.attempts,
            kind,
        }));
    }
}

/// 把一条"永久失败"的任务快照进死信表。
#[tracing::instrument(skip_all)]
async fn dead_letter_task(
    pool: &PgPool,
    delivery_id: Uuid,
    task_type: &str,
    newsletter_issue_id: Option<Uuid>,
    subscription_token: Option<&str>,
    recipient: &str,
    last_error: &str,
    attempts: i32
) -> Result<(), anyhow::Error> {
    sqlx::query!(
        r#"
        INSERT INTO dead_letter_queue
            (delivery_id, task_type, recipient, newsletter_issue_id,
             subscription_token, attempts, last_error)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
        delivery_id,
        task_type,
        recipient,
        newsletter_issue_id,
        subscription_token,
        attempts,          // ← 需要把 attempts 传进来（见下面说明）
        last_error,
    )
        .execute(pool)
        .await?;
    Ok(())
}
#[tracing::instrument(skip_all)]
async fn delete_task(
    pool: &PgPool,
    delivery_id: Uuid,
) -> Result<(), anyhow::Error> {
    sqlx::query!(
        r#"
        DELETE FROM email_delivery_queue
        WHERE delivery_id = $1
        "#,
        delivery_id
    )
        .execute(pool)
        .await?;
    Ok(())
}

/// 第 `attempt` 次失败后应该等多久再重试。
///
/// 两个部分：
///   - **指数退避**：`base * 2^attempt`，让低频失败恢复得快、高频失败不死磕；
///   - **full jitter**：在 `[0, backoff]` 里均匀取一个值。
///     没有抖动的话，一批同时失败的任务会在同一秒集体重试 ——
///     那只是把重试风暴推迟了，并没有消除。
///
/// 随机数发生器作为参数传入（而不是内部直接调 thread_rng），
/// 这样单元测试可以注入固定种子的 RNG，断言上下界和"确实在抖动"。
fn retry_delay<R: Rng>(attempt: i32, rng: &mut R) -> Duration {
    let exponent = attempt.max(0).min(20) as u32; // 截断指数，防止移位溢出
    let backoff_seconds = RETRY_BASE_SECONDS
        .saturating_mul(1u64 << exponent)
        .min(MAX_BACKOFF_SECONDS);

    // Duration 没有 From<u64>，所以要显式构造
    Duration::from_secs(rng.gen_range(0..=backoff_seconds))
}
/// 记录一次投递失败：把任务留在队列里，但把 next_retry_at 推到未来。
///
/// 注意这里用的是**退避时长**，不是租约期 —— 两者语义不同：
///   - 租约期（领取任务时设置）：防止"发信还没超时，别人就来抢"；
///   - 退避时长（失败后设置）：给暂时性故障留恢复时间，并避免重试风暴。
///
/// 退避时长由 retry_delay 算好再传进来：SQL 只负责 `now() + interval`，
/// 公式只存在于 Rust 一处，避免"SQL 里算一半、Rust 里算一半"。
async fn record_failure(
    pool: &PgPool,
    id: Uuid,
    attempt: i32,
    error: &str
) -> Result<(), anyhow::Error> {
    let delay = retry_delay(attempt, &mut rand::thread_rng());
    sqlx::query!(
        r#"
        UPDATE email_delivery_queue
        SET next_retry_at = now() + make_interval(secs => $2::float8),
            last_error = $3
        WHERE delivery_id = $1
        "#,
        id,
        delay.as_secs_f64(),
        error
    )
        .execute(pool)
        .await?;
    Ok(())
}
async fn worker_loop(
    pool: PgPool,
    email_client: EmailClient,
    base_url: &str,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<(), anyhow::Error> {
    loop {
        if *shutdown.borrow() {
            tracing::info!("Worker received shutdown signal, stopping after the current task.");
            return Ok(())
        }
        match try_execute_task(&pool, &email_client, base_url).await {
            Ok(ExecutionOutcome::EmptyQueue) => {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(10)) => {}
                    _ = shutdown.changed() => {}
                }
            }
            // 注意：投递失败**不会**走到这里 —— 失败已经被 record_failure
            // 记录成"等待退避"，并返回 Ok(TaskCompleted)。
            // 所以 Err 现在只代表基础设施故障（连不上数据库、SQL 出错等），
            // 这种情况下短暂等待再重试是合理的。
            Err(e) => {
                tracing::error!(error.cause_chain = ?e, error.message = %e, "Worker failed to execute a task");

                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                    _ = shutdown.changed() => {}
                }
            }
            Ok(ExecutionOutcome::TaskCompleted) => {}
        }
    }
}

pub async fn run_worker_until_stopped(configuration: Settings, shutdown: tokio::sync::watch::Receiver<bool>) -> Result<(), anyhow::Error> {
    let connection_pool = get_connection_pool(&configuration.database);

    let sender_email = configuration
        .email_client
        .sender()
        .expect("Invalid sender email address.");
    let timeout = configuration.email_client.timeout();
    let email_client = EmailClient::new(
        configuration.email_client.base_url.clone(),
        sender_email,
        configuration.email_client.authorization_token.clone(),
        timeout,
    );

    worker_loop(connection_pool, email_client, configuration.application.base_url.as_str(), shutdown).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::StdRng, SeedableRng};

    /// 上界 = base * 2^attempt（再受 MAX_BACKOFF_SECONDS 封顶）。
    #[test]
    fn retry_delay_respects_the_exponential_upper_bound() {
        // 固定种子 → 结果可复现，测试不会偶发失败
        let mut rng = StdRng::seed_from_u64(42);

        for attempt in 0..6 {
            let expected_cap = Duration::from_secs(
                RETRY_BASE_SECONDS * (1u64 << attempt),
            );
            for _ in 0..200 {
                let delay = retry_delay(attempt, &mut rng);
                assert!(
                    delay <= expected_cap,
                    "attempt={attempt} 的退避 {delay:?} 超过了上界 {expected_cap:?}"
                );
            }
        }
    }

    /// 指数增长到一定程度必须被 MAX_BACKOFF_SECONDS 封住。
    #[test]
    fn retry_delay_is_capped() {
        let mut rng = StdRng::seed_from_u64(7);
        let cap = Duration::from_secs(MAX_BACKOFF_SECONDS);

        // 指数非常大，如果不封顶这里会溢出/给出天文数字
        for attempt in [10, 20, 40, 100] {
            for _ in 0..100 {
                let delay = retry_delay(attempt, &mut rng);
                assert!(delay <= cap, "attempt={attempt} 的退避 {delay:?} 没有被封顶");
            }
        }
    }

    /// 同一 attempt 多次调用不应当总是同一个值 —— 这就是"抖动在起作用"。
    /// 没有它，一批同时失败的任务会在同一秒集体重试。
    #[test]
    fn retry_delay_actually_jitters() {
        let mut rng = StdRng::seed_from_u64(1);
        let values: Vec<Duration> = (0..50).map(|_| retry_delay(4, &mut rng)).collect();

        let all_same = values.windows(2).all(|w| w[0] == w[1]);
        assert!(!all_same, "退避没有抖动：一批任务会同时重试，形成重试风暴");
    }

    /// attempts 是"领取次数（含本次）"，所以第 1 次失败时它已经是 1。
    /// 这个测试把语义钉死，避免以后有人算错一次。
    #[test]
    fn retry_delay_grows_with_attempts() {
        let mut rng = StdRng::seed_from_u64(99);
        // 用足够多的样本比较"平均量级"：attempt 越大，上界越大
        let largest_for =
            |attempt: i32, rng: &mut StdRng| (0..500).map(|_| retry_delay(attempt, rng)).max();
        let a1 = largest_for(1, &mut rng).unwrap();
        let a3 = largest_for(3, &mut rng).unwrap();
        assert!(a3 > a1, "退避没有随 attempts 增长：a1={a1:?} a3={a3:?}");
    }
}