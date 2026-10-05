//! src/routes/subscriptions/persistence.rs
use crate::domain::{NewSubscriber, SubscriberStatus};
use chrono::Utc;
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
use sqlx::{Postgres, Transaction};
use std::error::Error;
use std::fmt::Formatter;
use uuid::Uuid;

/// 订阅接口的数据访问层：所有和 subscriptions / subscription_tokens 两张表
/// 打交道的 SQL 都集中在这里，handler 只负责编排。
#[tracing::instrument(
    name = "Saving new subscriber details in the database",
    skip(new_subscriber, transaction)
)]
pub async fn insert_subscriber(
    transaction: &mut Transaction<'_, Postgres>,
    new_subscriber: &NewSubscriber,
) -> Result<SubscriberRecord, sqlx::Error> {
    let subscriber_id = Uuid::new_v4();
    // 注意 ON CONFLICT ... DO UPDATE ... WHERE 的一个关键语义：
    //   WHERE 不成立时【什么都不会发生】，而且 RETURNING 返回【0 行】——
    //   不是返回被跳过的旧行。所以必须用 fetch_optional，
    //   用 None 表示「这一行已存在，而且因为已经是 confirmed 被 WHERE 挡下了」。
    let row = sqlx::query!(
        r#"
        INSERT INTO subscriptions (id, email, name, subscribed_at, status)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (email)
        DO UPDATE SET name = EXCLUDED.name
        WHERE subscriptions.status <> $6
        RETURNING id, status
        "#,
        subscriber_id,
        new_subscriber.email.as_ref(),
        new_subscriber.name.as_ref(),
        Utc::now(),
        SubscriberStatus::PendingConfirmation.as_str(),
        SubscriberStatus::Confirmed.as_str(),
    )
    .fetch_optional(transaction)
    .await
    .map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        e
    })?;

    match row {
        // 插入成功，或命中冲突且完成了 UPDATE
        Some(r) => {
            let status = SubscriberStatus::try_from(r.status).map_err(|e| {
                sqlx::Error::Decode(Box::new(e))
            })?;
            Ok(SubscriberRecord { id: r.id, status })
        }
        // 没有返回行：只可能是「已存在且是 confirmed，被 WHERE 挡下了」。
        // Confirmed 分支不会用到 id，这里用本次生成的 uuid 占位即可。
        None => Ok(SubscriberRecord {
            id: subscriber_id,
            status: SubscriberStatus::Confirmed,
        }),
    }
}

/// 订阅者的落库结果：handler 需要它来分叉后续动作。
pub struct SubscriberRecord {
    pub id: Uuid,
    pub status: SubscriberStatus,
}

/// 生成长度为 25 个字符且大小写敏感订阅令牌
pub fn generate_subscription_token() -> String {
    let mut rng = thread_rng();
    std::iter::repeat_with(|| rng.sample(Alphanumeric))
        .map(char::from)
        .take(25)
        .collect()
}

#[tracing::instrument(
    name = "Store subscription token in the database",
    skip(subscriber_token, transaction)
)]
pub async fn store_token(
    transaction: &mut Transaction<'_, Postgres>,
    subscriber_id: Uuid,
    subscriber_token: &str,
) -> Result<(), StoreTokenError> {
    // 一条 upsert 完成「轮换」：
    //   - 该订阅者还没有 token → 插入新行
    //   - 已有 token           → 覆盖 subscription_token 与 expire_at（旧 token 立即失效）
    // 依赖迁移里建的唯一索引 subscription_tokens_subscriber_id_unique 作为冲突目标。
    // 不需要先 DELETE：并发时由数据库保证同一个 subscriber_id 不会留下两行。
    sqlx::query!(
        r#"
        INSERT INTO subscription_tokens (subscription_token, subscriber_id, expire_at)
        VALUES ($1, $2, $3)
        ON CONFLICT (subscriber_id) DO UPDATE
        SET subscription_token = EXCLUDED.subscription_token,
            expire_at = EXCLUDED.expire_at
        "#,
        subscriber_token,
        subscriber_id,
        Utc::now() + chrono::Duration::days(7)
    )
    .execute(&mut *transaction)
    .await
    .map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        StoreTokenError(e)
    })?;
    Ok(())
}

pub struct StoreTokenError(pub(crate) sqlx::Error); // 包装器

impl std::fmt::Debug for StoreTokenError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        error_chain_fmt(self, f)
    }
}

impl std::fmt::Display for StoreTokenError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "A database error was encountered while \
            trying to store a subscription token."
        )
    }
}

impl Error for StoreTokenError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

/// 这个 helper 目前被 subscriptions / login 两边共用，
/// 按约定先保持原来的位置不动（见 mod.rs 的 re-export）。
pub fn error_chain_fmt(e: &impl Error, f: &mut Formatter<'_>) -> std::fmt::Result {
    writeln!(f, "{}\n", e)?;
    let mut current = e.source();
    while let Some(cause) = current {
        writeln!(f, "Caused by:\n\t{}", cause)?;
        current = cause.source();
    }
    Ok(())
}
