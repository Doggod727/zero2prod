//! tests/api/subscribers.rs
use crate::helpers::{assert_is_redirect_to, spawn_app, TestApp};

/// 固定的基准时间，保证测试可复现。
const BASE: &str = "2026-01-01T00:00:00+00:00";

/// 造 `count` 个已确认订阅者。
/// 每 3 行共用一个时间戳 —— 故意制造“同一秒多行”，
/// 用来验证游标里的 id tiebreaker 真的在起作用。
async fn create_confirmed_subscribers(app: &TestApp, count: i32) {
    sqlx::query!(
        r#"
        INSERT INTO subscriptions (id, email, name, subscribed_at, status)
        SELECT gen_random_uuid(),
               'paged' || g || '@example.com',
               'user ' || g,
               $1::timestamptz + make_interval(secs => (g / 3)),
               'confirmed'
        FROM generate_series(1, $2) g
        "#,
        chrono::DateTime::parse_from_rfc3339(BASE).unwrap(),
        count
    )
    .execute(&app.db_pool)
    .await
    .expect("Failed to seed subscribers.");
}

/// 从 HTML 里抠出“下一页”链接的 href（没有则返回 None）。
///
/// 注意：**不能用 linkify::LinkFinder** —— 它只认带 scheme 的绝对 URL，
/// 而我们渲染的是相对路径 `/admin/subscribers?...`，它会一个都找不到。
const NEXT_PAGE_PREFIX: &str = "href=\"/admin/subscribers?";

fn next_page_link(html: &str) -> Option<String> {
    let start = html.find(NEXT_PAGE_PREFIX)? + "href=\"".len();
    let end = html[start..].find('"')? + start;
    Some(html[start..end].to_owned())
}

/// 从 href（`/admin/subscribers?after=..&after_id=..`）取出可拼给请求的 query 部分。
fn query_of(href: &str) -> &str {
    let idx = href.find('?').expect("next page link should carry a query");
    &href[idx..]
}

/// 这一页 HTML 里出现的所有邮箱（`paged<数字>@example.com`）。
fn emails_in(html: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = html[cursor..].find("paged") {
        let start = cursor + offset;
        let end = match html[start..].find('@') {
            Some(i) => start + i,
            None => break,
        };
        found.push(html[start..end].to_owned());
        cursor = end;
    }
    found
}

/// 多页遍历：不重叠、不遗漏。
#[tokio::test]
async fn subscribers_list_paginates_without_gaps_or_overlaps() {
    let app = spawn_app().await;
    app.test_user.login(&app).await;
    create_confirmed_subscribers(&app, 45).await;

    let mut seen: Vec<String> = Vec::new();
    let mut query = String::new();
    let mut pages = 0;

    loop {
        let html = app.get_subscribers_html(&query).await;
        pages += 1;
        seen.extend(emails_in(&html));

        match next_page_link(&html) {
            Some(href) => {
                query = query_of(&href).to_owned();
                assert!(pages < 10, "分页似乎没有终止");
            }
            None => break,
        }
    }

    let total_before_dedup = seen.len();
    seen.sort();
    seen.dedup();

    assert_eq!(
        seen.len(),
        45,
        "遍历完所有页后应当恰好看到 45 个不同的订阅者（不重叠、不遗漏），实际 {}",
        seen.len()
    );
    assert_eq!(total_before_dedup, 45, "不应当有任何订阅者在两页里重复出现");
    assert_eq!(pages, 3, "45 条、每页 20 条 → 应当是 3 页");
}

/// 最后一页没有“下一页”链接。
#[tokio::test]
async fn the_last_page_has_no_next_link() {
    let app = spawn_app().await;
    app.test_user.login(&app).await;
    // 只有 5 条 → 一页装得下 → 第一页就是最后一页
    create_confirmed_subscribers(&app, 5).await;

    let html = app.get_subscribers_html("").await;
    assert_eq!(emails_in(&html).len(), 5, "5 条应当都在这一页");
    assert!(
        next_page_link(&html).is_none(),
        "只有一页时不应当出现下一页链接"
    );
}

/// 同秒订阅者：验证游标里的 id 起了 tiebreaker 的作用。
/// 21 条数据只跨 7 个时间戳 → 第一页会在“同一秒中间”切断，
/// 如果游标少了 id，第二页就会重复或漏掉同一秒内的行。
#[tokio::test]
async fn subscribers_list_handles_rows_sharing_the_same_timestamp() {
    let app = spawn_app().await;
    app.test_user.login(&app).await;
    create_confirmed_subscribers(&app, 21).await;

    let first = app.get_subscribers_html("").await;
    let first_page_emails = emails_in(&first);
    assert_eq!(first_page_emails.len(), 20, "第一页应当是 20 条");

    let href = next_page_link(&first).expect("21 条应当还有下一页");
    let second = app.get_subscribers_html(query_of(&href)).await;
    let second_page_emails = emails_in(&second);
    assert_eq!(second_page_emails.len(), 1, "第二页应当只剩 1 条");

    for email in &second_page_emails {
        assert!(
            !first_page_emails.contains(email),
            "同一秒边界处出现了重复：{email} 同时出现在两页"
        );
    }

    let mut all: Vec<String> = first_page_emails
        .into_iter()
        .chain(second_page_emails)
        .collect();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 21, "两页合起来应当恰好覆盖全部 21 条");
}

/// 未登录访问会被中间件挡住。
#[tokio::test]
async fn you_must_be_logged_in_to_see_the_subscribers_list() {
    let app = spawn_app().await;

    let response = app.get_subscribers_page("").await;
    assert_is_redirect_to(&response, "/login");
}

/// 游标只给一半 → 400（而不是静默回到第一页）。
#[tokio::test]
async fn a_half_supplied_cursor_is_rejected() {
    let app = spawn_app().await;
    app.test_user.login(&app).await;

    let only_after = app
        .get_subscribers_page("?after=2026-01-01T00:00:00Z")
        .await;
    assert_eq!(
        400,
        only_after.status().as_u16(),
        "只给 after 不给 after_id 应当被拒绝，而不是静默返回第一页"
    );

    let only_id = app
        .get_subscribers_page("?after_id=0192abcd-0000-0000-0000-000000000000")
        .await;
    assert_eq!(
        400,
        only_id.status().as_u16(),
        "只给 after_id 不给 after 应当被拒绝"
    );
}
