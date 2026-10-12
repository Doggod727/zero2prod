//! src/routes/subscriptions/subscribe/mod.rs
mod get;
mod persistence;
mod post;

pub use get::subscription_form;
pub use persistence::{insert_subscriber, store_token, StoreTokenError};
pub use post::{subscribe, SubscriberError};
