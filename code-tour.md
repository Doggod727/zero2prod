# 从 main 出发的代码导览（一步步深入）

> 方法：自顶向下追调用栈 + 两条主线（启动流程 / 请求流程）。
> 规则：**每站先回答 4 个问题，再往下走**；只跟本项目模块，遇到 actix/tokio/sqlx 内部就停（除非它是面试点）。

## 每站必答的 4 问
1. **输入**是什么？（参数从哪来）
2. **输出**是什么？（返回什么、给谁）
3. 有什么**副作用**？（写库 / 发请求 / 改 session）
4. **失败**会怎样？（返回什么状态码 / 谁重试）

每读完一条链路，产出：**一张链路图 + 一张决策卡**。

---

## 第一部分：启动流程（服务怎么跑起来）

1. **`main.rs::main`**
   - 初始化 tracing → 读配置 → `Application::build` → `tokio::spawn` 两个任务（API + worker）→ `select!` + `report_exit`。
   - 自问：为什么要 spawn 两个任务？worker 为什么不塞进 `Application::build`？
2. **`startup.rs::Application::build`**
   - 建连接池、构造 `EmailClient`、bind listener、调用 `run`、返回 `{port, server}`。
   - 自问：`EmailClient` 为什么在启动时构造一次？
3. **`startup.rs::run`**
   - 组装中间件（`TracingLogger` / `FlashMessages` / `Session(Redis)`）、注册路由、`/admin` scope + `from_fn(reject_anonymous_users)`、`app_data` 注入。
   - 自问：中间件顺序有影响吗？`app_data` 和 `web::Data` 什么关系？（八股：依赖注入）
4. **`configurations.rs::get_configurations`**
   - 分层配置：`base.yaml` + `{APP_ENVIRONMENT}.yaml` + `APP_` 环境变量。
   - 自问：为什么配置要分环境、还要支持环境变量覆盖？
5. **`issue_delivery_worker.rs::run_worker_until_stopped` → `worker_loop`**
   - 先只看循环骨架（空队列 sleep 10s / 出错 sleep 1s），细节后面回来。

---

## 第二部分：最简单的请求流程（`POST /subscriptions`）

6. **`routes/subscriptions.rs::subscribe`**
   - `web::Form` 提取 → `TryFrom<FormData> for NewSubscriber`（校验）→ **事务**：`insert_subscriber` + `store_token` → `commit` → `send_confirmation_email`。
   - 自问：为什么用事务？**为什么先 commit 再发邮件**？（八股：事务边界、外部 I/O 别包在事务里）
7. **`domain/new_subscriber.rs` / `subscriber_name.rs` / `subscriber_email.rs`**
   - 类型驱动验证：`parse` 构造，字段私有，`AsRef` 只读。
   - 自问：为什么不用 `String` 直接传？（八股：类型安全 / 不变量）
8. **`email_client.rs::send_email`**
   - `reqwest::Client` + 超时 + `error_for_status()`。
   - 自问：为什么客户端要复用（连接池）？超时设多长？
9. **`routes/subscriptions_confirm.rs::confirm`**
   - `web::Query` 取 token → 查订阅者 → `UPDATE status='confirmed'`。
   - 自问：为什么用独立 token 表而不是直接把 id 放 URL？

---

## 第三部分：认证与会话

10. **`routes/login/get.rs::login_form` / `post.rs::login`**
    - Form → `validate_credentials` → `session.insert_user_id` → 303。
    - 自问：失败为什么返回 303 跳回登录页而不是 401？（配合 Flash）
11. **`authentication.rs::validate_credentials` / `verify_password_hash`**
    - Argon2 校验；用户不存在时也跑一次假 hash（防用户枚举）；`spawn_blocking` 避免阻塞异步线程。
    - 自问：为什么哈希是 CPU 密集、要丢到阻塞线程池？（八股：异步运行时）
12. **`session_state.rs::TypedSession`**
    - 包装 `Session`，`from_request` 取 `req.get_session()`。
13. **`authentication.rs::reject_anonymous_users` + `UserId`**
    - 中间件：读 session → 有 user_id 就 `extensions_mut().insert(UserId)` 放行；否则 303。
    - 自问：和 Actix `Service/Transform` 的区别？（八股：中间件）
14. **`routes/admin/dashboard.rs::admin_dashboard`**
    - `web::ReqData<UserId>` 取出（中间件注入的）→ 查 username → 渲染。
15. **`routes/admin/password/post.rs::change_password`** + **`utils.rs::{e500,e400,see_other}`**
    - 校验旧密码 → 改密；错误用 `ResponseError` / 工具函数映射状态码。

---

## 第四部分：核心链路——幂等 + 队列 + 后台 worker（重点）

16. **`routes/admin/newsletters/get.rs::newsletter_form`**
    - 生成隐藏的 `idempotency_key` 放进表单。
    - 自问：为什么每次渲染一个新 key？
17. **`routes/admin/newsletters/post.rs::publish_newsletter`**
    - `try_processing` → `insert_newsletter_issue` → `enqueue_delivery_tasks` → `save_response`（**这里 commit**）→ 请求内 `loop { try_execute_task }`。
    - 自问：哪一步提交了事务？为什么 loop 用的是**新事务**？（上一个问题我们讨论过）
18. **`idempotency/key.rs::IdempotencyKey` + `persistence.rs::{try_processing, save_response, get_saved_response}`**
    - `ON CONFLICT DO NOTHING` 抢键；保存/重放 `HttpResponse`；`save_response` 里 `commit`。
    - 自问：为什么幂等键用 PG 不用 Redis？
19. **`issue_delivery_worker.rs::{dequeue_task, get_issue, try_execute_task, delete_task}`**
    - `SELECT ... FOR UPDATE SKIP LOCKED LIMIT 1` → 发信 → 成功 `DELETE`+commit，失败回滚保留。
    - 自问：`SKIP LOCKED` 解决什么？为什么是“至少一次”？
20. 回看 **`main.rs` 的 worker spawn**：理解“请求内尽力 + 后台兜底”。

---

## 第五部分：工程化与测试

21. **`tests/api/helpers.rs::spawn_app` / `configure_database`**
    - 每用例随机库名 + `CREATE DATABASE` + `migrate!`；wiremock 当邮件服务；带 cookie 的 reqwest。
    - 自问：为什么每个测试一个独立数据库？
22. **`tests/api/newsletter.rs`**
    - 幂等、并发、重试三类用例怎么断言。
23. **`migrations/*`**
    - 按时间顺序看表是怎么演进出来的（尤其 idempotency / newsletter_issues / issue_delivery_queue）。
24. **`.github/workflows/ci.yml`**
    - fmt / clippy / test / sqlx offline。

---

## 学完后应有的产出（检查清单）

- [ ] 一张启动流程图（main → 服务监听 → worker）
- [ ] 一张 Newsletter 派发链路图（含事务边界与 commit 位置）
- [ ] 至少 5 张决策卡（幂等键、SKIP LOCKED、会话、中间件、事务边界）
- [ ] 能脱稿讲 2 分钟“一次发布请求到底发生了什么”
