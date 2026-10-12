//! src/routes/subscriptions/persistence.rs
use crate::domain::{NewSubscriber, SubscriberStatus};
use crate::utils::error_chain_fmt;
use chrono::Utc;
use sqlx::{Postgres, Transaction};
use std::error::Error;
use std::fmt::Formatter;
use uuid::Uuid;

#[tracing::instrument(
    name = "Saving new subscriber details in the database",
    skip(new_subscriber, transaction)
)]
pub async fn insert_subscriber(
    transaction: &mut Transaction<'_, Postgres>,
    new_subscriber: &NewSubscriber,
) -> Result<SubscriberRecord, sqlx::Error> {
    // 唯一约束是部分索引 UNIQUE (email) WHERE status <> 'unsubscribed'，
    // 语义是"同一邮箱最多一行活跃订阅"。它把四种情况都切对了：
    //
    //   表里现状              老行在索引里?   结果
    //   没有这个邮箱           —              INSERT
    //   pending_confirmation   在             DO UPDATE（复用老行）
    //   confirmed              在             DO UPDATE → 被 WHERE 挡下
    //   unsubscribed           【不在】        INSERT（老行作为历史留着）
    //
    // ⚠️ 冲突目标是"已存在的行在不在索引里"，不是"新行长什么样"。
    //
    // ⚠️ ON CONFLICT 的谓词必须和索引谓词一致，否则报
    //    "there is no unique or exclusion constraint matching the ON CONFLICT
    //     specification"（这个报错不会提示是谓词写漏了）。
    //
    // ⚠️ DO UPDATE 只改 SET 里列出的列 —— 漏写 status 会让退订过的人
    //    name 变了、status 仍停在 unsubscribed，静默且不报错。
    //
    // ⚠️ WHERE 不成立时 RETURNING 返回【0 行】（不是旧行），所以下面用
    //    fetch_optional，None 表示"已是 confirmed"。
    let subscriber_id = Uuid::new_v4();
    let row = sqlx::query!(
        r#"
        INSERT INTO subscriptions (id, email, name, subscribed_at, status)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (email) WHERE status <> 'unsubscribed'
        DO UPDATE SET name = EXCLUDED.name,
                      status = EXCLUDED.status
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
        Some(r) => {
            let status = SubscriberStatus::try_from(r.status)
                .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
            Ok(SubscriberRecord { id: r.id, status })
        }
        // 只可能是"已 confirmed，被 DO UPDATE 的 WHERE 挡下"。该分支不用 id，占位即可。
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

#[tracing::instrument(
    name = "Store subscription token in the database",
    skip(subscriber_token, transaction)
)]
pub async fn store_token(
    transaction: &mut Transaction<'_, Postgres>,
    subscriber_id: Uuid,
    subscriber_token: &str,
) -> Result<(), StoreTokenError> {
    // 一条 upsert 完成轮换：已有 token 就覆盖，旧 token 立即失效。
    // 冲突目标 subscription_tokens_subscriber_id_unique 由迁移建立；
    // 并发下由数据库保证同一个 subscriber_id 不会留下两行，不需要先 DELETE。
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

pub struct StoreTokenError(pub(crate) sqlx::Error);

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
