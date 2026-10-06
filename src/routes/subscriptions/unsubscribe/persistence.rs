//! src/routes/subscriptions/unsubscribe/persistence.rs
//!
//! 退订链路的 SQL 集中在这里。
use crate::domain::SubscriberStatus;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// 数据库投影：status 保持 String，读出来之后再转枚举。
///
/// 为什么不直接写 SubscriberIdentity、也不给枚举加 #[sqlx(try_from = "String")]：
///   `query_as!` 走的是编译期类型检查，它要求每一列能【被证明】映射到字段类型，
///   而 `#[sqlx(...)]` 属性只在 derive(FromRow) 路径下生效，宏路径不认。
///   所以这里沿用 insert_subscriber 的既有做法：投影用 String，转换在 Rust 侧显式做。
///
/// 好处是失败方式正确：数据库里出现不认识的状态值会变成 Decode 错误（500 + 日志），
/// 而不是被静默当成某个合法值。
struct SubscriberRow {
    id: Uuid,
    email: String,
    status: String,
}

/// token → 订阅者身份。
///
/// 两个 handler 都用它，但需要的字段不同：
///   GET  需要 status —— 要渲染"链接无效 / 已退订 / 确认退订"三种页面，
///                      不能对任何 token 都发一个"确认退订"按钮；
///   POST 需要 email  —— 队列里只存 subscription_token，worker 拿不到收件人，
///                      所以必须在入队之前把 email 取出来。
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

/// 把订阅者标记为已退订。返回 true 表示【这一次真的改了状态】。
///
/// ⚠️ 必须用 execute + rows_affected：
///   没有 RETURNING 子句的 UPDATE 输出的结果集是【空的】，
///   fetch_optional 会永远返回 None —— 表现是"退订成功但通知永远不入队"，且不报错。
///   rows_affected 数的是真正被改动的行，和 RETURNING 无关。
///
/// 返回 bool 而不是三态：调用方在 "已经是 unsubscribed" 和 "没有这个人" 两种情况下
/// 要做的事完全一样（什么都不做），多一个状态只会让调用方多写一个空分支。
/// 收 &mut Transaction 而不是 &PgPool：调用方（unsubscribe 的 POST）本来就要
/// 把"改状态 / 写 token / 入队通知"放进同一个事务，收池子会逼它在外面再开一个事务、
/// 或者干脆放弃原子性。
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

/// 确保这个订阅者有一个退订 token，并把【库里实际生效的那个】还回去。
///
/// 语义是【确保存在】，不是【写入】—— 名如其分地叫 store 只是为了和 store_token 对称。
///
/// ⚠️ 三个必须这么写的理由：
///
/// 1. 冲突时【不轮换】。用户反复提交订阅表单时 token 保持稳定，
///    否则收件箱里那些旧邮件里的退订链接会全部失效 ——
///    而那恰恰是用户最可能去点的那封。退订是"撤销权限"的动作，
///    旧链接永远能撤销，是正确语义，不是漏洞。
///
/// 2. `DO UPDATE SET unsubscription_token = unsubscription_tokens.unsubscription_token`
///    看着像废话，但它把值【赋成它自己】：值不变，却因此能 RETURNING 出行来。
///    实测：`DO NOTHING` 在冲突时 RETURNING 返回 0 行；
///    上面这个写法在冲突时返回库里已有的 token。
///    为什么要多此一举 —— 为了让调用方【拿到权威值】，见第 3 点。
///
/// 3. 参数拿走所有权、返回值还回来。因为冲突时传给它的值是会被丢弃的，
///    handler 手上那个 token 和库里的可能【不是同一个】。
///    把签名设计成 String -> Result<String>，"不小心用了自己那个值"就写不出来
///    （变量已经被 move 走了，编译器会拦）。
///    如果只是想稳住 token 本身，用 &str -> Result<()> 也够；
///    这里多这一步是为了让"通知邮件里的链接必须来自库"这件事无法被忽略。
///
/// 收 &mut Transaction 而不是 &PgPool：调用方（subscribe / unsubscribe）本来就开着事务，
/// 放进同一个事务能保证"取值"和"写入"在同一个快照里，没有 READ COMMITTED 下的缝隙。
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

/// 入队一封"你已退订"的通知，并顺手清掉他还没发出去的 newsletter 任务。
///
/// ⚠️ DELETE 那一步不能省：退订和"上一次群发已经入队但还没发完"完全可能同时发生。
///    少了它，用户退订之后还会收到那封群发邮件 —— 所有退订 bug 里最伤信任的一种。
///
/// 冲突目标用 email_delivery_queue_pending_notification_unique，
/// 它覆盖 confirmation 和 unsubscription 两种通知任务。用 DO UPDATE（不是 DO NOTHING）
/// 是因为要顺便处理"退订 → 又订阅"的竞态：那一行可能已经变成一条确认任务，
/// 把它顶掉换成退订通知的 token，比留着一条矛盾的确认邮件正确。
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
