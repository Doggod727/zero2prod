//! src/routes/subscriptions/persistence.rs
use crate::domain::{NewSubscriber, SubscriberStatus};
use crate::utils::error_chain_fmt;
use chrono::Utc;
use sqlx::{Postgres, Transaction};
use std::error::Error;
use std::fmt::Formatter;
use uuid::Uuid;

/// 订阅接口的数据访问层：所有和 subscriptions / subscription_tokens 两张表
/// 打交道的 SQL 都集中在这里，handler 只负责编排。
pub async fn insert_subscriber(
    transaction: &mut Transaction<'_, Postgres>,
    new_subscriber: &NewSubscriber,
) -> Result<SubscriberRecord, sqlx::Error> {
    let subscriber_id = Uuid::new_v4();

    // ============================================================
    // 这里【不能】用一条 ON CONFLICT 搞定，原因值得记下来
    // ============================================================
    //
    // 邮箱唯一约束是一个【部分】索引：
    //     UNIQUE (email) WHERE status <> 'unsubscribed'
    // 意思是"同一邮箱最多一行活跃订阅"，退订过的行想留几行都行。
    //
    // 于是"退订过的人重新订阅"这个场景最自然的写法会坏掉：
    //
    //     INSERT ... VALUES (..., 'pending_confirmation')
    //     ON CONFLICT (email) WHERE status <> 'unsubscribed'
    //     DO UPDATE SET ...
    //
    // 为什么坏 —— 实测过（同一张表、同一批数据）：
    //     退订后 = unsubscribed
    //     这条语句返回 = pending_confirmation      ← 看着对了
    //     结果：rows=2，老行仍然是 unsubscribed     ← 其实插了【新行】
    //
    // 根因：部分索引的谓词是对【候选新行】判断的，不是对【已存在的冲突行】。
    //   新行是 'pending_confirmation' → 满足谓词 → Postgres 认为它就是这个索引
    //   该管的那一行 → 走 INSERT 分支，根本不进 DO UPDATE。
    //   `ON CONFLICT (email) WHERE ...` 里的 WHERE 只负责"让索引能被推断出来"，
    //   它决定不了"选中哪一行来更新"。
    //   结果就是：老行留着（等于这个邮箱有两行），而 RETURNING 返回的是
    //   那个没人会去读的新行。
    //
    // 所以改成两步、语义与自己写的意图一一对应：
    //   ① 先按【活跃行】更新 —— 命中就是"这个人已经存在，刷新名字/状态"
    //   ② 没有活跃行才 INSERT —— 新订阅者，或退订过的人重新回来
    //
    // 代价（要如实说）：
    //   · 常见路径多一次 round-trip（① 未命中时才走 ②）；
    //   · 两个【真正并发】的首次订阅，② 会有一个撞唯一索引报 500。
    //     这一点不如 ON CONFLICT 优雅 —— 后者在这种竞态下也是 500
    //     （DO UPDATE 撞主键），所以严格说并没有变差，但也没有变好。
    //     真要消除它得做"捕获唯一冲突后重试一次"，等有真实需求再说。
    // ============================================================

    // ① 已经有【待确认】的订阅？刷新它。
    //
    // ⚠️ 这里的谓词和 ② 里那个 ON CONFLICT 的谓词【故意不一样】，两者别抄混：
    //
    //   ① 要选的是"可以被这次提交刷新的行" = 只认 'pending_confirmation'。
    //      - 'confirmed'   不能选：已确认的用户不该被降级回待确认（会重发确认邮件）
    //      - 'unsubscribed' 不能选：退订那行是【历史】，要原样留着
    //
    //   ② 的冲突目标要写"活跃"的定义 = status <> 'unsubscribed'。
    //      那是 email 唯一索引的谓词，Postgres 要求冲突推断必须能推出索引谓词，
    //      所以那里一个字符都不能改。
    //
    //   两个谓词看着像同一个概念，其实回答的是两个不同的问题：
    //     ① "这一行能不能被我改写"
    //     ② "这个索引管不管这一行"
    //   曾经把 ① 写成 status <> 'confirmed'（从旧的 ON CONFLICT 版本抄来的），
    //   结果它命中了退订行、把唯一那条退订历史复活成了 pending_confirmation ——
    //   测试断言"两行都在"直接失败，表里只剩一行。
    let updated = sqlx::query!(
        r#"
        UPDATE subscriptions
        SET name = $2,
            status = $3
        WHERE email = $1
          AND status = $4
        RETURNING id, status
        "#,
        new_subscriber.email.as_ref(),
        new_subscriber.name.as_ref(),
        SubscriberStatus::PendingConfirmation.as_str(),
        SubscriberStatus::PendingConfirmation.as_str(),
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        e
    })?;

    if let Some(r) = updated {
        let status =
            SubscriberStatus::try_from(r.status).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        return Ok(SubscriberRecord { id: r.id, status });
    }

    // ② 没有【待确认】的行 → 插入或告知"已确认"。
    //
    // 能走到这里有三种情况：
    //   · 这个邮箱从没订阅过                     → INSERT 成功，正常发确认邮件
    //   · 这个邮箱退订过                         → INSERT 成功，【新建一行】，
    //                                             退订那行原样留着作为历史
    //   · 这个邮箱已经是 'confirmed'             → INSERT 会撞上部分唯一索引？
    //     不，恰恰相反：新行是 pending_confirmation、满足索引谓词，而已有行也
    //     满足谓词，所以【是】真的唯一冲突 → ON CONFLICT DO NOTHING → 0 行。
    //     下面那个 None 分支会在"活跃"行里读到它，返回 status=confirmed，
    //     handler 据此走"已订阅"那条路（不重发、不降级）。
    //
    // 用 ON CONFLICT (email) WHERE ... DO NOTHING 而不是裸 INSERT：
    // 唯一的竞态是"两个并发请求同时走 ②"，那时后到的那个会撞上
    // subscriptions_active_email_unique。用 DO NOTHING 把它变成 0 行，
    // 然后由下面的 match 决定怎么处理 —— 总比抛一个数据库错误给用户好。
    let inserted = sqlx::query!(
        r#"
        INSERT INTO subscriptions (id, email, name, subscribed_at, status)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (email) WHERE status <> 'unsubscribed' DO NOTHING
        RETURNING id, status
        "#,
        subscriber_id,
        new_subscriber.email.as_ref(),
        new_subscriber.name.as_ref(),
        Utc::now(),
        SubscriberStatus::PendingConfirmation.as_str(),
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        e
    })?;

    match inserted {
        Some(r) => {
            let status = SubscriberStatus::try_from(r.status)
                .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
            Ok(SubscriberRecord { id: r.id, status })
        }
        // DO NOTHING 命中：就在这两条语句之间，另一个并发请求把同一个邮箱
        // 插成了活跃行。再读一次它即可 —— 结果和"我先进去"完全一样。
        None => {
            let existing = sqlx::query!(
                r#"
                SELECT id, status FROM subscriptions
                WHERE email = $1 AND status <> 'unsubscribed'
                "#,
                new_subscriber.email.as_ref(),
            )
            .fetch_one(&mut **transaction)
            .await?;
            let status = SubscriberStatus::try_from(existing.status)
                .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
            Ok(SubscriberRecord {
                id: existing.id,
                status,
            })
        }
    }
}
/// 订阅者的落库结果：handler 需要它来分叉后续动作。
pub struct SubscriberRecord {
    pub id: Uuid,
    pub status: SubscriberStatus,
}

// generate_subscription_token 已删除：它和 crate::utils::generate_token 是同一份实现，
// 留着会有两个"生成 token"的入口，以后改强度必然漏掉一处。

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
