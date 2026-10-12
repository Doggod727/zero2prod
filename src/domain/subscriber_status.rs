//! src/domain/subscriber_status.rs

/// 订阅者在生命周期中的状态。
///
/// 用枚举而不是 String：拼错编译不过；新增状态时所有 match 会被编译器强制补全。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriberStatus {
    PendingConfirmation,
    Confirmed,
    Unsubscribed,
}

impl SubscriberStatus {
    /// 数据库里用的字面量，集中在此，避免散落在 SQL 与 match 之间。
    pub fn as_str(&self) -> &'static str {
        match self {
            SubscriberStatus::PendingConfirmation => "pending_confirmation",
            SubscriberStatus::Confirmed => "confirmed",
            SubscriberStatus::Unsubscribed => "unsubscribed",
        }
    }

    /// 待确认的用户才需要（重新）发送确认邮件；已确认的用户不应该被降级。
    pub fn needs_confirmation_email(&self) -> bool {
        matches!(self, SubscriberStatus::PendingConfirmation)
    }

    /// 还在收信的人。"活跃"是一个领域概念，定义只此一处
    /// （SQL 里仍然写 status = 'confirmed'，靠 as_str() 保持同一个字面量）。
    pub fn is_active(&self) -> bool {
        matches!(self, SubscriberStatus::Confirmed)
    }
}

/// 从数据库读出来的文本映射回枚举；未知值直接报错，而不是静默归类。
impl TryFrom<String> for SubscriberStatus {
    type Error = UnknownSubscriberStatus;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "pending_confirmation" => Ok(SubscriberStatus::PendingConfirmation),
            "confirmed" => Ok(SubscriberStatus::Confirmed),
            "unsubscribed" => Ok(SubscriberStatus::Unsubscribed),
            other => Err(UnknownSubscriberStatus(other.to_owned())),
        }
    }
}

impl std::fmt::Display for SubscriberStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 数据库里出现了我们不认识的状态值：说明有迁移/代码不同步，必须显式失败。
#[derive(Debug)]
pub struct UnknownSubscriberStatus(pub String);

impl std::fmt::Display for UnknownSubscriberStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown subscriber status: {}", self.0)
    }
}

impl std::error::Error for UnknownSubscriberStatus {}
