//! src/routes/admin/newsletters/mod.rs
mod get;
mod post;

pub use get::newsletter_form;
pub use post::publish_newsletter;
