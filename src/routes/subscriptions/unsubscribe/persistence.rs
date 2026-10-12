//! src/routes/subscriptions/unsubscribe/persistence.rs
use crate::domain::SubscriberStatus;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// 数据库投影：status 保持 String，转换在 Rust 侧做。
/// `query_as!` 走编译期检查，不接受 `#[sqlx(try_from = "String")]`（那个只在
/// derive(FromRow) 路径生效）。好处是遇到未知状态值会变成 Decode 错误而不是被静默归类。
struct SubscriberRow {
    id: Uuid,
    email: String,
    status: String,
}

/// token → 订阅者身份。
///
/// GET 要 status（决定渲染哪个页面），POST 要 email（队列里只有 token，
/// worker 拿不到收件人，必须在入队前取出来）。
pub struct SubscriberIdentity {
    pub id: Uuid,
    pub email: String,
    pub status: SubscriberStatus,
}

pub async fn find_subscriber_by_unsubscription_token(
    pool: &PgPool,
    token: &str,
) -> Result<Option<SubscriberIdentity>, sqlx::Error> {
    let row = sqlx::query_as!(
        SubscriberRow,
        r#"
        SELECT s.id, s.email, s.status
        FROM unsubscription_tokens t
        JOIN subscriptions s ON s.id = t.subscriber_id
        WHERE t.unsubscription_token = $1
        "#,
        token
    )
    .fetch_optional(pool)
    .await?;

    row.map(|r| {
        let status =
            SubscriberStatus::try_from(r.status).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        Ok(SubscriberIdentity {
            id: r.id,
            email: r.email,
            status,
        })
    })
    .transpose()
}

/// 标记为已退订。返回 true 表示【这一次真的改了状态】。
///
/// ⚠️ 用 execute + rows_affected，不用 fetch_optional：没有 RETURNING 的 UPDATE
///    结果集是空的，fetch_optional 永远返回 None（表现是"退订成功但通知永不入队"，不报错）。
///
/// 返回 bool 而不是三态：调用方对"已退订"和"没这个人"的动作完全一样。
/// 收 &mut Transaction 是为了让调用方把三张表的写入放进同一个事务。
pub async fn mark_as_unsubscribed(
    transaction: &mut Transaction<'_, Postgres>,
    subscriber_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        r#"
        UPDATE subscriptions
        SET status = 'unsubscribed'
        WHERE id = $1 AND status != 'unsubscribed'
        "#,
        subscriber_id
    )
    .execute(&mut **transaction)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// 确保这个订阅者有一个退订 token，返回【库里实际生效的那个】。
///
/// ⚠️ 冲突时不轮换（`SET t = t`）：旧邮件里的退订链接必须一直有效，
///    轮换会让收件箱里所有旧链接失效 —— 而那正是用户最可能点的那封。
///    赋成自己看似废话，但它让 RETURNING 能返回库里的旧值
///    （`DO NOTHING` 在冲突时 RETURNING 返回 0 行）。
///
/// ⚠️ 参数拿走所有权、返回值还回来：冲突时传进来的值会被丢弃，
///    handler 手上那个和库里的可能不是同一个。签名这样设计，
///    "不小心用自己那个值"就写不出来（变量已被 move）。
pub async fn store_unsubscription_token(
    transaction: &mut Transaction<'_, Postgres>,
    subscriber_id: Uuid,
    unsubscription_token: String,
) -> Result<String, sqlx::Error> {
    let token = sqlx::query!(
        r#"
        INSERT INTO unsubscription_tokens (unsubscription_token, subscriber_id)
        VALUES ($1, $2)
        ON CONFLICT (subscriber_id)
        DO UPDATE SET unsubscription_token = unsubscription_tokens.unsubscription_token
        RETURNING unsubscription_token
        "#,
        unsubscription_token,
        subscriber_id
    )
    .fetch_one(&mut **transaction)
    .await?;
    Ok(token.unsubscription_token)
}

/// 入队"你已退订"通知，并清掉他还没发出去的 newsletter 任务。
///
/// ⚠️ DELETE 不能省：退订和"上次群发已入队但没发完"完全可能同时发生，
///    少了它用户退订后还会收到那封群发邮件。
///
/// 冲突目标覆盖 confirmation 和 unsubscription 两种通知任务。用 DO UPDATE
/// （不是 DO NOTHING）是为了处理"退订 → 又订阅"的竞态：把那一行可能已经变成的
/// 确认任务顶掉，比留着一条矛盾的确认邮件正确。
pub async fn enqueue_unsubscription_notice(
    transaction: &mut Transaction<'_, Postgres>,
    recipient: &str,
    unsubscription_token: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        DELETE FROM email_delivery_queue
        WHERE recipient = $1 AND task_type = 'newsletter'
        "#,
        recipient
    )
    .execute(&mut **transaction)
    .await?;

    sqlx::query!(
        r#"
        INSERT INTO email_delivery_queue (task_type, recipient, subscription_token)
        VALUES ('unsubscription', $1, $2)
        ON CONFLICT (recipient) WHERE task_type IN ('confirmation', 'unsubscription')
        DO UPDATE SET task_type = EXCLUDED.task_type,
                      subscription_token = EXCLUDED.subscription_token,
                      attempts = 0,
                      next_retry_at = now(),
                      last_error = NULL
        "#,
        recipient,
        unsubscription_token
    )
    .execute(&mut **transaction)
    .await?;
    Ok(())
}
