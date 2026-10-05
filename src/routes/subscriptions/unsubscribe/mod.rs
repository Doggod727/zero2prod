//! src/routes/subscriptions/unsubscribe/mod.rs
//!
//! 退订（待实现）。计划：
//!   - GET  /subscriptions/unsubscribe?token=...   用户点邮件里的链接
//!   - POST /subscriptions/unsubscribe             二次确认（可选）
//!   - 复用 subscription_tokens 那套一次性 token + 7 天过期
//!   - 需要给 subscriptions.status 增加 'unsubscribed'，并同步 CHECK 约束