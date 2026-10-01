# 邮件订阅服务系统 · 面试拆解手册

> 目标：把项目从“我写过”变成“我能讲清每一个决策”。
> 面试真正问的是**决策与权衡**，不是“你写了哪些文件”。

---

## 一、四步法

1. **全局图**：一张图说清请求怎么进来、数据怎么流、有哪些组件。
2. **链路卡**：按“业务链路”（不是按文件）拆，每条链路能 60 秒讲完。
3. **决策卡**：每个关键技术选择，回答“为什么是它、代价是什么”。
4. **反向出题**：把自己当面试官，预判 15~20 个追问，准备好答案。
   最后把八股**挂到**这些卡片上（八股 → 项目证据）。

---

## 二、全局图（自己手画一版，画完就记住一半）

```
Browser ──HTTP──> Actix Web App
   中间件链: TracingLogger → FlashMessages → Session(Redis)
   ┌ /admin scope: reject_anonymous_users（未登录 → 303 /login）
   │
   ├ handlers: home / subscriptions / subscriptions/confirm
   │           /login(get,post)
   │           /admin/dashboard | /admin/password | /admin/newsletters
   │
   ├ PostgreSQL:
   │    subscriptions, subscription_tokens, users,
   │    idempotency, newsletter_issues, issue_delivery_queue
   ├ Redis: session（后续：限流计数）
   └ Email API（Postmark 风格；测试用 wiremock 拦截）

后台任务（tokio::spawn）: issue_delivery_worker
   └ 循环从 issue_delivery_queue 取任务 → 发邮件
```

一句话版本：**“一个 Actix 服务 + 一个后台 worker，共享 PG 与 Redis，通过数据库任务队列解耦发信。”**

---

## 三、链路卡模板 + 已填示例

模板：
- **触发**：
- **步骤**（输入 → 处理 → 输出）：
- **涉及组件/文件**：
- **关键代码位置**：
- **异常/边界**：

### 示例：Newsletter 派发
- **触发**：管理员在 `/admin/newsletters` 提交表单（含隐藏 `idempotency_key`）。
- **步骤**：
  1. `reject_anonymous_users` 校验登录 → 注入 `UserId`；
  2. `try_processing`：`INSERT idempotency ... ON CONFLICT DO NOTHING`，拿到事务；
  3. 同一事务：`INSERT newsletter_issues`；`INSERT issue_delivery_queue SELECT email FROM subscriptions WHERE status='confirmed'`；
  4. `save_response`：把 303 写回 idempotency 并 **commit**（issue + 队列 + 幂等一起落库）；
  5. 本请求内循环 `try_execute_task` 直到空；后台 worker 兜底。
- **输出**：303 重定向 + Flash；邮件尽力/异步发出。
- **位置**：`routes/admin/newsletters/post.rs:34-66`、`idempotency/persistence.rs:57-98`、`issue_delivery_worker.rs:47-133`。
- **异常**：某封发送失败 → 该请求 500、任务保留；同 key 重试 → 重放 303 并继续投递剩余任务。

> 其余链路（订阅确认 / 登录会话 / 改密 / 限流 / 分页）各写一张，格式相同。

---

## 四、决策卡（最有价值，面试官爱问）

模板：
- **问题**：
- **候选方案**：
- **最终选择**：
- **理由**：
- **代价 / 风险**：
- **如何验证**：
- **可能的追问**：

### 决策 1：幂等键存哪里？
- 问题：重复 / 并发提交如何保证只投递一次？
- 候选：Redis（带 TTL） / PostgreSQL 表。
- 选择：PostgreSQL，主键 `(user_id, idempotency_key)` + 保存完整响应。
- 理由：① 要和 `newsletter_issues`、`issue_delivery_queue` 在**同一事务**原子提交；② 要能原样重放 `HttpResponse`；③ Redis 是内存库，可能淘汰 / 丢键，一旦处理中丢键就会重复执行。
- 代价 / 风险：表会膨胀 → 需要 TTL + 定期清理。
- 验证：`newsletter_creation_is_idempotent`、`concurrent_form_submission_is_handled_gracefully`。
- 追问：过期怎么处理？→ 见决策 4。

### 决策 2：任务队列怎么让多 worker 不抢同一条？
- 问题：多实例 worker 并发消费，既不重复也不互相阻塞。
- 候选：Redis 分布式锁 / PG 行锁 / PG `FOR UPDATE SKIP LOCKED`。
- 选择：PG `SELECT ... FOR UPDATE SKIP LOCKED LIMIT 1`。
- 理由：不引入额外组件；“领取即加锁”，并发 worker 各拿各的；失败回滚，任务保留。
- 代价：**至少一次**语义（发送成功但删除前崩溃会重复）；需要幂等兜底。
- 验证：`concurrent_form_submission_is_handled_gracefully`、`transient_errors...`。
- 追问：为什么不用 Redis 队列？→ 要和业务写入同事务、少一个组件；SKIP LOCKED 已足够。

### 决策 3：会话存哪里？中间件怎么做的？
- 问题：登录状态跨请求、跨 worker 保持；未登录统一拦截。
- 选择：`actix-session` + Redis 存储；`from_fn(reject_anonymous_users)` 挂 `/admin` scope。
- 理由：多 worker 无状态水平扩展；中间件集中鉴权，handler 只管业务。
- 代价：多一个 Redis 依赖；中间件里做了一次 session 反序列化。
- 追问：和 Actix 的 `Service/Transform` 有何区别？→ `from_fn` 是函数式包装，前者是底层 trait。

### 决策 4：幂等键过期怎么处理？
- 问题：不清理会膨胀；清理不当会误删“处理中”的键导致重复。
- 选择：TTL + 访问时惰性接管（`ON CONFLICT DO UPDATE ... WHERE create_at < now()-ttl`）+ 后台 sweeper。
- 边界：存在且未过期且有响应 → 重放；存在但响应为 NULL（处理中）→ 409；不存在 → 当作新请求。
- 追问：TTL 设多大？→ 必须大于最大处理时间（如 24h）。

### 决策 5：错误怎么返回？
- 问题：不同失败原因要映射不同状态码，且日志要能追根因。
- 选择：模块化错误枚举 + `ResponseError` 映射状态码；`thiserror` 生成 Display，`source()` 保留错误链。
- 追问：为什么不统一一个 `AppError`？→ 也可，但当前按模块更内聚。

---

## 五、三个“能讲深”的点（优先准备）

1. **幂等 + DB 任务队列 + `SKIP LOCKED`**：至少一次、重试风暴、并发消费。
2. **会话认证 + 自定义中间件 + Argon2**：横向扩展、集中鉴权。
3. **MVCC / 隔离级别**：结合 worker 的锁、丢失更新实验（RC vs RR）。

准备三档时长：**30 秒 / 2 分钟 / 5 分钟**，都能张口就来。

---

## 六、反向出题清单（把自己当面试官）

1. 为什么幂等键用 PG 不用 Redis？
2. 并发同 key 请求会怎样？处理中的请求返回什么？
3. 邮件发送失败怎么办？会不会重复？重复了怎么办？
4. 为什么用 `SKIP LOCKED`？不加会怎样？多 worker 会不会抢同一条？
5. 后台 worker 崩了任务会丢吗？为什么？
6. 既然有后台 worker，为什么请求里还要 drain 一遍？
7. session 为什么必须放 Redis 而不是进程内存？
8. `reject_anonymous_users` 怎么实现的？和 `Service/Transform` 区别？
9. 密码为什么不能明文 / 不能裸 SHA？Argon2 的 salt 有什么用？
10. 为什么默认 READ COMMITTED？会不会不可重复读 / 脏读？
11. `newsletter_issues` 和 `issue_delivery_queue` 为什么要分两张表？
12. 幂等事务里为什么要 `ON CONFLICT DO NOTHING` 而不是先查后插？
13. 测试怎么做到用例之间互不干扰？（每用例独立库）
14. 为什么用 wiremock？测的是契约的哪些部分？
15. 如果订阅者有几百万，`INSERT ... SELECT` 入队和分页会有什么问题？怎么优化？
16. 如果要支持“撤回已发布 newsletter”，你会怎么改表？
17. 现在 Redis 只做 session，什么时候才该上缓存？
18. 限流为什么用 Redis 而不是本地计数器？
19. 线程 / 协程 / Tokio 任务的区别？worker 为什么用 `tokio::spawn`？
20. 这个项目你最后悔的设计是什么？如果重做会怎么改？

---

## 七、八股 ↔ 项目映射表（让八股“落”在项目上）

| 八股主题 | 项目里的证据 / 话术 |
|---|---|
| 索引 / 最左前缀 / 回表 | 订阅者分页 + `EXPLAIN` 前后对比 |
| 事务 / MVCC / 隔离级别 | 幂等保存、`SKIP LOCKED`、RC vs RR 丢失更新实验 |
| 至少一次 / 幂等 | 幂等键 + 响应重放 + 任务队列 |
| 缓存 / 限流 / 分布式锁 | session(Redis)、登录限流(Redis+Lua)、用 `SKIP LOCKED` 代替分布式锁 |
| 进程 / 线程 / 异步 | `tokio::spawn` 后台 worker、Actix worker 模型 |
| HTTP / 状态码语义 | 303 重定向、401/403、429、409 |
| 密码学基础 | Argon2 + salt + PHC 字符串 |
| 服务可观测性 | tracing span / instrument / request-id |

> 用法：面试官问八股时，先给定义，再补一句“**我在项目里是怎么用的 / 踩过什么坑**”，立刻从背书变成实战。
