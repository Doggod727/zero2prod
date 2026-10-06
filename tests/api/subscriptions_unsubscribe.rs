//! tests/api/subscriptions_unsubscribe.rs
//!
//! 退订链路的集成测试。
//!
//! 这一套里最重要的不是"退订能成功"（那个写起来最容易），
//! 而是三条容易在真实环境炸、但本地单测发现不了的契约：
//!   1. GET 不改状态（邮件客户端会预取链接）
//!   2. 邮件带 List-Unsubscribe / List-Unsubscribe-Post（RFC 8058 合规）
//!   3. 一键退订的 body 不是我们的表单结构，handler 不能依赖 body
use crate::helpers::{assert_is_redirect_to, spawn_app, TestApp};

/// 订阅一个测试用邮箱，返回它的退订链接（从确认邮件的 List-Unsubscribe 头里抠）。
///
/// 这里用 List-Unsubscribe 头而不是正文里那个链接，是为了顺手把
/// "头确实发出去了、而且和正文指向同一个端点"这件事一起验证掉。
async fn subscribe_and_get_unsubscribe_link(app: &TestApp, email: &str) -> reqwest::Url {
    app.mock_email_server_returning_200().await;

    let body = format!("name=le%20guin&email={}", email.replace('@', "%40"));
    app.post_subscriptions(body).await;

    let requests = app.emails_sent().await;
    assert_eq!(requests.len(), 1, "订阅应该只产生一封确认邮件");
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    app.unsubscribe_url_from(&body)
        .expect("确认邮件里没有 List-Unsubscribe 头 —— RFC 8058 的合规入口缺失了")
}

/// 把最后一个订阅者确认掉：取最新那封邮件里的确认链接，GET 一下。
async fn confirm_latest_subscriber(app: &TestApp) {
    let emails = app.emails_sent().await;
    let last = emails.last().expect("还没有任何邮件");
    let body: serde_json::Value = serde_json::from_slice(&last.body).unwrap();
    let link = app.confirmation_link_from(&body);
    app.get_url(link).await.error_for_status().unwrap();
}

/// 取这个邮箱【当前活跃】那一行的状态。
///
/// ⚠️ 不能写成 `WHERE email = $1` + fetch_one：
///    退订过又回来订阅之后，同一个邮箱会有两行
///    （一行 'unsubscribed' 作为历史，一行活跃的）。
///    那样查会随机抓一行，测试就变成"有时绿有时红"——
///    而这正是它一开始失败的原因：抓到了历史那行。
async fn status_of(app: &TestApp, email: &str) -> String {
    sqlx::query!(
        "SELECT status FROM subscriptions
         WHERE email = $1 AND status <> 'unsubscribed'
         ORDER BY subscribed_at DESC
         LIMIT 1",
        email
    )
    .fetch_one(&app.db_pool)
    .await
    .expect("这个邮箱没有活跃订阅")
    .status
}

/// 这个邮箱的【全部】行状态，按字典序排好。
///
/// 为什么需要一个独立的助手：退订之后这个邮箱就【没有活跃行了】，
/// 所以 status_of（它只找活跃行）会 panic。而"没有活跃行"本身
/// 恰恰是我们要断言的事实 —— 用一个"查全部行"的视图来表达，
/// 比给 status_of 加一堆 Option 更好读。
async fn all_statuses(app: &TestApp, email: &str) -> Vec<String> {
    let mut statuses: Vec<String> =
        sqlx::query!("SELECT status FROM subscriptions WHERE email = $1", email)
            .fetch_all(&app.db_pool)
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.status)
            .collect();
    statuses.sort();
    statuses
}
/// 队列里还压着几条退订通知。
///
/// 只在"发信失败、任务留在队列里等退避"时才有意义 ——
/// 正常路径下退订请求内的 drain 循环会把它发掉。
async fn queued_unsubscribe_notices(app: &TestApp) -> i64 {
    sqlx::query!(
        "SELECT count(*) AS count FROM email_delivery_queue WHERE task_type = 'unsubscription'"
    )
    .fetch_one(&app.db_pool)
    .await
    .unwrap()
    .count
    .unwrap_or(0)
}

// ============================================================
// ① 预取安全：GET 绝对不能改状态
// ============================================================
//
// 这个测试存在的理由：Gmail / Outlook 在【展示】邮件时会预先抓取邮件里的链接
// （防钓鱼扫描、生成预览、决定要不要显示原生退订按钮）。
// 如果 GET 里执行退订，用户还没看到邮件就已经被退订了，而且他永远不会知道原因。
//
// 这个 bug 在本地手动测试里【发现不了】—— 你自己点链接时是"人"在点，
// 只有邮件客户端那种"先抓一遍"的行为才会触发。
#[tokio::test]
async fn a_get_on_the_unsubscribe_link_does_not_change_the_subscription() {
    let app = spawn_app().await;
    let email = "prefetch@example.com";
    let link = subscribe_and_get_unsubscribe_link(&app, email).await;
    confirm_latest_subscriber(&app).await;
    assert_eq!(status_of(&app, email).await, "confirmed");

    // 模拟邮件客户端预取：反复 GET，而且带一堆查询参数（有些扫描器会加）
    for _ in 0..3 {
        let response = app.get_url(link.clone()).await;
        assert_eq!(response.status().as_u16(), 200);
    }

    assert_eq!(
        status_of(&app, email).await,
        "confirmed",
        "GET 改了状态 —— 邮件客户端预取会把用户静默退订掉"
    );
    assert_eq!(
        queued_unsubscribe_notices(&app).await,
        0,
        "GET 竟然入队了退订通知"
    );
}

// ============================================================
// ② 完整链路：确认 → 点退订页 → POST → 状态变更
// ============================================================
#[tokio::test]
async fn confirming_then_unsubscribing_flips_the_status() {
    let app = spawn_app().await;
    let email = "full-flow@example.com";
    let link = subscribe_and_get_unsubscribe_link(&app, email).await;
    confirm_latest_subscriber(&app).await;

    // GET 渲染确认页（应该有按钮，但不动状态）
    let page = app.get_url(link.clone()).await.text().await.unwrap();
    assert!(page.contains("确认退订"), "确认页没有退订按钮: {page}");
    assert_eq!(status_of(&app, email).await, "confirmed");

    // POST 才执行
    let token = link
        .query_pairs()
        .find(|(k, _)| k == "unsubscription_token")
        .map(|(_, v)| v.to_string())
        .expect("退订链接里没有 token");
    let response = app.post_unsubscribe(&token, false).await;
    assert_eq!(response.status().as_u16(), 200);

    assert_eq!(all_statuses(&app, email).await, vec!["unsubscribed"]);
}

// ============================================================
// ③ RFC 8058 一键退订：body 不是我们的表单结构
// ============================================================
//
// Gmail 发的 body 是 `List-Unsubscribe=One-Click`，不是 name/email 那种结构。
// 如果 POST handler 用 web::Form<FormData> 去解析它，会直接 400 ——
// 而这是【生产环境才会暴露】的失败：本地你永远是用浏览器点按钮。
#[tokio::test]
async fn a_one_click_post_unsubscribes_without_a_form_body() {
    let app = spawn_app().await;
    let email = "one-click@example.com";
    let link = subscribe_and_get_unsubscribe_link(&app, email).await;
    confirm_latest_subscriber(&app).await;

    let token = link
        .query_pairs()
        .find(|(k, _)| k == "unsubscription_token")
        .map(|(_, v)| v.to_string())
        .unwrap();

    // 模拟邮件客户端：POST + 那个 body + 【不带 Content-Type】
    let response = app.post_unsubscribe(&token, true).await;
    assert_eq!(
        response.status().as_u16(),
        200,
        "一键退订被拒了 —— 客户端发的 body 不是我们的表单结构，handler 不能依赖 body"
    );
    assert_eq!(all_statuses(&app, email).await, vec!["unsubscribed"]);
}

// ============================================================
// ④ 退订之后不再收到 newsletter
// ============================================================
#[tokio::test]
async fn unsubscribed_users_do_not_receive_newsletters() {
    let app = spawn_app().await;
    use fake::faker::internet::en::SafeEmail;
    use fake::Fake;
    let email: String = SafeEmail().fake();
    let link = subscribe_and_get_unsubscribe_link(&app, &email).await;
    confirm_latest_subscriber(&app).await;

    let token = link
        .query_pairs()
        .find(|(k, _)| k == "unsubscription_token")
        .map(|(_, v)| v.to_string())
        .unwrap();
    app.post_unsubscribe(&token, false).await;
    assert_eq!(all_statuses(&app, &email).await, vec!["unsubscribed"]);

    // 发一期 newsletter
    app.test_user.login(&app).await;
    let body = serde_json::json!({
        "title": "Newsletter title",
        "text_content": "Text body",
        "html_content": "<p>HTML body</p>",
        "idempotency_key": uuid::Uuid::new_v4().to_string(),
    });
    let response = app.post_newsletters(body).await;
    assert_is_redirect_to(&response, "/admin/newsletters");

    // 队列里不该有给他的 newsletter 任务。
    // 注意这里查【队列】而不是查"有没有发信"：
    // 队列是原因，发信是结果；查原因能更早、更确定地失败。
    let queued = sqlx::query!(
        "SELECT count(*) AS count FROM email_delivery_queue WHERE task_type = 'newsletter'"
    )
    .fetch_one(&app.db_pool)
    .await
    .unwrap()
    .count
    .unwrap_or(0);
    assert_eq!(queued, 0, "退订的用户仍然被排进了 newsletter 队列");

    // 顺带确认退订后他的 newsletter 任务是【真被删掉了】：
    // 上面那个 DELETE 是为了处理"退订时上一次群发还没发完"的竞态。
    let leftovers = sqlx::query!(
        "SELECT count(*) AS count FROM email_delivery_queue WHERE recipient = $1",
        email
    )
    .fetch_one(&app.db_pool)
    .await
    .unwrap()
    .count
    .unwrap_or(0);
    assert_eq!(leftovers, 0, "退订后队列里还留着他的任务");
}

// ============================================================
// ⑤ 幂等：重复退订不重发通知
// ============================================================
#[tokio::test]
async fn unsubscribing_twice_is_idempotent() {
    let app = spawn_app().await;
    let email = "idempotent@example.com";
    let link = subscribe_and_get_unsubscribe_link(&app, email).await;
    confirm_latest_subscriber(&app).await;
    let token = link
        .query_pairs()
        .find(|(k, _)| k == "unsubscription_token")
        .map(|(_, v)| v.to_string())
        .unwrap();

    // ⚠️ 断言"已发出的邮件数"，不要断言"队列里有几条任务"。
    //    退订请求里那个 drain 循环会把队列排空去发通知，
    //    所以请求返回之后队列本来就该是空的 ——
    //    拿队列数量当断言，测到的是"退避状态"，不是业务语义。
    let before = app.emails_sent().await.len();
    let response = app.post_unsubscribe(&token, false).await;
    assert_eq!(response.status().as_u16(), 200);

    let after = app.emails_sent().await;
    assert_eq!(
        after.len(),
        before + 1,
        "第一次退订应该立刻发出一封『你已退订』通知"
    );
    let subject = after.last().unwrap();
    let body: serde_json::Value = serde_json::from_slice(&subject.body).unwrap();
    assert_eq!(
        body["Subject"], "You have been unsubscribed",
        "发出的不是退订通知"
    );

    // 第二次：状态已经是 unsubscribed，不该再发一封
    let response = app.post_unsubscribe(&token, false).await;
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(
        app.emails_sent().await.len(),
        before + 1,
        "重复退订又发了一封通知 —— 用户会反复收到『你已退订』"
    );
}

// ============================================================
// ⑥ 无效 token：200 而不是 4xx
// ============================================================
//
// 为什么不给 404："链接无效"是【页面内容】，不是协议错误。
// 给 4xx 会让邮件客户端在收件箱里挂一个错误图标，
// 用户看到的也是浏览器默认错误页，完全没有上下文。
#[tokio::test]
async fn an_unknown_unsubscribe_token_renders_a_page_with_a_200() {
    let app = spawn_app().await;

    let response = app.get_unsubscribe_page("totally-made-up-token").await;
    assert_eq!(
        response.status().as_u16(),
        200,
        "无效 token 返回了 4xx —— 邮件客户端会显示错误图标"
    );
    let body = response.text().await.unwrap();
    assert!(body.contains("链接无效"), "提示页文案不对: {body}");

    let response = app.post_unsubscribe("totally-made-up-token", false).await;
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(queued_unsubscribe_notices(&app).await, 0);
}

// ============================================================
// ⑦ 邮件合规头：List-Unsubscribe + List-Unsubscribe-Post
// ============================================================
//
// ⚠️ 这两个头必须出现在 JSON body 的 Headers 数组里。
//    如果当成 HTTP 请求头发出去，Postmark 会静默忽略：
//    邮件照发、没有报错、但收件箱里不会出现原生退订按钮。
//    这是最典型的"合规静默失效"——本地 mock 测得过，真发出去不生效。
#[tokio::test]
async fn emails_advertise_one_click_unsubscribe_in_the_json_body() {
    let app = spawn_app().await;
    let email = "headers@example.com";
    let _ = subscribe_and_get_unsubscribe_link(&app, email).await;

    let requests = app.emails_sent().await;
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();

    let headers = body["Headers"].as_array().expect("Headers 不是数组");
    let names: Vec<&str> = headers.iter().filter_map(|h| h["Name"].as_str()).collect();
    assert!(
        names.contains(&"List-Unsubscribe"),
        "缺 List-Unsubscribe: {names:?}"
    );
    assert!(
        names.contains(&"List-Unsubscribe-Post"),
        "缺 List-Unsubscribe-Post（没有它客户端不会显示一键退订按钮）: {names:?}"
    );

    // 一键退订的授权声明：值必须完全一致，否则客户端不认。
    let post_header = headers
        .iter()
        .find(|h| h["Name"] == "List-Unsubscribe-Post")
        .unwrap();
    assert_eq!(post_header["Value"], "List-Unsubscribe=One-Click");

    // 反例：绝不能用 HTTP 头发出去。
    // 遍历而不是 headers.get(...)：wiremock 的 http crate 版本和 reqwest 不同，
    // 直接构造 HeaderName 要跨 crate 转换。
    let sent_as_http_header = requests[0]
        .headers
        .iter()
        .any(|(name, _)| name.as_str().eq_ignore_ascii_case("list-unsubscribe"));
    assert!(
        !sent_as_http_header,
        "List-Unsubscribe 被当成 HTTP 头发了出去，Postmark 会忽略它"
    );

    // 头里的 URL 必须指向 POST 那个端点（GET 那个是确认页，一键退订打上去会 405）
    let url = app.unsubscribe_url_from(&body).expect("抠不出退订 URL");
    assert_eq!(url.path(), "/subscriptions/unsubscribe");
    assert!(url.query_pairs().any(|(k, _)| k == "unsubscription_token"));
}

// ============================================================
// ⑧ 退订后还能重新订阅（Unsubscribed 不是终态）
// ============================================================
#[tokio::test]
async fn an_unsubscribed_user_can_subscribe_again() {
    let app = spawn_app().await;
    let email = "come-back@example.com";
    let link = subscribe_and_get_unsubscribe_link(&app, email).await;
    confirm_latest_subscriber(&app).await;
    let token = link
        .query_pairs()
        .find(|(k, _)| k == "unsubscription_token")
        .map(|(_, v)| v.to_string())
        .unwrap();
    app.post_unsubscribe(&token, false).await;
    assert_eq!(all_statuses(&app, email).await, vec!["unsubscribed"]);

    // 重新提交订阅表单
    let body = format!("name=le%20guin&email={}", email.replace('@', "%40"));
    let response = app.post_subscriptions(body).await;
    assert_is_redirect_to(&response, "/subscriptions");
    assert_eq!(
        status_of(&app, email).await,
        "pending_confirmation",
        "退订过的人回不来了 —— 他主动订阅却拿不到确认邮件"
    );

    // 这里顺带钉住"重新订阅是【新建一行】"这个决策。
    //
    // 唯一约束是 UNIQUE (email) WHERE status <> 'unsubscribed' ——
    // 它表达的是"同一邮箱最多一行【活跃】订阅"，退订过的行可以留任意多行。
    // 于是重新订阅时最自然的做法是"复用老行、把它改回 pending_confirmation"。
    // 那个做法在 SQL 层看着更省事，但语义上有个矛盾：
    // 如果以后有人查历史（"这个邮箱什么时候退订过"），复用老行就把那段
    // 历史抹掉了 —— 而 'unsubscribed' 这个状态存在的意义正是"留下痕迹"。
    //
    // 所以这里断言两行都在：一行 unsubscribed（历史）+ 一行 pending_confirmation（现在）。
    // 如果哪天有人改成复用老行，这条会失败 —— 那正是我们想要的提醒。
    let rows = sqlx::query!(
        "SELECT status FROM subscriptions WHERE email = $1 ORDER BY status",
        email
    )
    .fetch_all(&app.db_pool)
    .await
    .unwrap();
    let statuses: Vec<&str> = rows.iter().map(|r| r.status.as_str()).collect();
    assert_eq!(
        statuses,
        vec!["pending_confirmation", "unsubscribed"],
        "重新订阅应该是新建一行、保留退订那一行作为历史"
    );
}
