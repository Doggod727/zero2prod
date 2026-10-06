//! src/routes/subscriptions/subscribe/mod.rs
mod get;
mod persistence;
mod post;

pub use get::subscription_form;
pub use persistence::{insert_subscriber, store_token, StoreTokenError};
pub use post::{subscribe, SubscriberError};

// 说明：
//   - error_chain_fmt 已经挪到 crate::utils（原来放在 persistence.rs 里，
//     却被 login 反向 import，依赖方向是反的）；
//   - send_confirmation_email 已删除。它从"改为走队列发确认邮件"那一刻起
//     就没人调用了，是死代码 —— 留着会让人以为有两条发信路径。
