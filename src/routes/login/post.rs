//! src/routes/login/post.rs

use crate::authentication::{validate_credentials, AuthError, Credentials};
use crate::routes::error_chain_fmt;
use actix_web::error::InternalError;
use actix_web::http::header::LOCATION;
use actix_web::web;
use actix_web::HttpResponse;
use secrecy::{Secret};
use sqlx::PgPool;
use std::fmt::Formatter;
use actix_web_flash_messages::FlashMessage;
use crate::rate_limiting::{LoginRateLimiter, WINDOW_SECONDS};
use crate::session_state::TypedSession;
#[derive(serde::Deserialize)]
pub struct FormData {
    username: String,
    password: Secret<String>,
}
#[derive(thiserror::Error)]
pub enum LoginError {
    #[error("Authentication failed")]
    AuthError(#[source] anyhow::Error),
    #[error("Something went wrong")]
    UnexpectedError(#[from] anyhow::Error),
}

impl std::fmt::Debug for LoginError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        error_chain_fmt(self, f)
    }
}
#[tracing::instrument(
    skip(form, pool, session, rate_limiter),
    fields(uesrname=tracing::field::Empty, user_id=tracing::field::Empty)
)]
pub async fn login(
    form: web::Form<FormData>,
    pool: web::Data<PgPool>,
    session: TypedSession,
    rate_limiter: web::Data<LoginRateLimiter>
) -> Result<HttpResponse, InternalError<LoginError>> {
    // 1）先限流
    match rate_limiter.try_acquire(&form.username).await {
        Ok(true) => {/*放行，继续检验密码*/},
        Ok(false) => {
            // 访问超限
            return Ok(HttpResponse::TooManyRequests()
                .insert_header(("Retry-After", WINDOW_SECONDS.to_string()))
                .finish());
        }
        Err(e) => {
            // fail-open: 可用性优先，继续去校验密码，但是要有日志
            tracing::error!(error.cause_chain = ?e, error.message = %e, "Rate limiter unavailable, allowing the request (fail-open)");
        }
    }
    let credentials = Credentials {
        username: form.0.username,
        password: form.0.password,
    };
    tracing::Span::current().record("username", &tracing::field::display(&credentials.username));
    match validate_credentials(credentials, &pool).await {
        Ok(user_id) => {
            tracing::Span::current().record("user_id", &tracing::field::display(&user_id));
            session.renew();
            session.insert_user_id(user_id)
                .map_err(|e| login_redirect(LoginError::UnexpectedError(e.into())))?;
            Ok(HttpResponse::SeeOther()
                .insert_header((LOCATION, "/admin/dashboard"))
                .finish())
        }
        Err(e) => {
            let e = match e {
                AuthError::InvalidCredentials(_) => LoginError::AuthError(e.into()),
                AuthError::UnexpectedError(_) => LoginError::UnexpectedError(e.into()),
            };
            FlashMessage::error(e.to_string()).send();
            let response = HttpResponse::SeeOther()
                .insert_header((
                    LOCATION,
                    "/login",
                ))
                .finish();
            Err(InternalError::from_response(e, response))
        }
    }
}

fn login_redirect(e: LoginError) -> InternalError<LoginError> {
    FlashMessage::error(e.to_string()).send();
    let response = HttpResponse::SeeOther()
        .insert_header((LOCATION, "/login"))
        .finish();
    InternalError::from_response(e, response)
}