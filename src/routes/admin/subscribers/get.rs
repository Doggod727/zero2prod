//! scr/routes/admin/subscribers/get.rs
use crate::utils::e500;
use actix_web::http::header::ContentType;
use actix_web::{web, HttpResponse};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::fmt::Write;
use uuid::Uuid;

const PAGE_SIZE: i64 = 20; // 一页的大小

#[derive(serde::Deserialize)]
pub struct Parameters {
    after: Option<DateTime<Utc>>,
    after_id: Option<Uuid>,
}

pub struct Record {
    id: Uuid,
    email: String,
    name: String,
    subscribed_at: DateTime<Utc>,
}
pub async fn subscribers_list(
    parameters: web::Query<Parameters>,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, actix_web::Error> {
    let mut rows = match parameters.0 {
        Parameters {
            after: None,
            after_id: None,
        } => {
            // 无游标查询
            get_first_page(&pool).await.map_err(e500)?
        }
        Parameters {
            after: Some(subscribed_at),
            after_id: Some(id),
        } => {
            // 有游标的查询
            get_next_page(&pool, subscribed_at, id)
                .await
                .map_err(e500)?
        }
        _other => {
            // 其他所有情况
            return Ok(HttpResponse::BadRequest().finish());
        }
    };
    let has_next = rows.len() > PAGE_SIZE as usize;
    let next_cursor = if has_next {
        rows.pop();
        rows.last().map(|r| (r.subscribed_at, r.id))
    } else {
        None
    };
    // 有下一页才去渲染链接
    let next_page_link = next_cursor.map(|(subscribed_at, id)| {
        // ⚠️ 不要用 to_rfc3339()：它会生成 `2026-01-01T00:00:08+00:00`，
        // 而 query string 里的 `+` 会被解析成【空格】——
        // 服务端收到的就变成 `2026-01-01T00:00:08 00:00`，DateTime 解析失败。
        //
        // 用 `Z` 结尾的 UTC 形式（RFC3339 允许，chrono 也能解析），就不含 `+`。
        format!(
            "/admin/subscribers?after={}&after_id={}",
            subscribed_at.format("%Y-%m-%dT%H:%M:%S%.fZ"),
            id
        )
    });
    // 渲染表格行
    let mut rows_html = String::new();
    for row in &rows {
        let _ = writeln!(
            rows_html,
            "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
            row.email, row.name, row.subscribed_at
        );
    }
    let next_page_html = match &next_page_link {
        Some(link) => format!("<p><a href=\"{link}\">Next page</a></p>"),
        None => String::new(),
    };
    Ok(HttpResponse::Ok()
        .content_type(ContentType::html())
        .body(format!(
            include_str!("subscribers.html"),
            rows_html = rows_html,
            next_page_html = next_page_html
        )))
}

async fn get_first_page(pool: &PgPool) -> Result<Vec<Record>, sqlx::Error> {
    let result = sqlx::query_as!(
        Record,
        r#"
        SELECT id, email, name, subscribed_at
        FROM subscriptions
        WHERE status = 'confirmed'
        ORDER BY subscribed_at DESC, id DESC
        LIMIT $1
        "#,
        PAGE_SIZE + 1i64
    )
    .fetch_all(pool)
    .await?;
    Ok(result)
}
async fn get_next_page(
    pool: &PgPool,
    subscribed_at: DateTime<Utc>,
    id: Uuid,
) -> Result<Vec<Record>, sqlx::Error> {
    let result = sqlx::query_as!(
        Record,
        r#"
        SELECT id, email, name, subscribed_at
        FROM subscriptions
        WHERE status = 'confirmed'
            AND (subscribed_at, id) < ($1, $2)
        ORDER BY subscribed_at DESC, id DESC
        LIMIT $3
        "#,
        subscribed_at,
        id,
        PAGE_SIZE + 1
    )
    .fetch_all(pool)
    .await?;
    Ok(result)
}
