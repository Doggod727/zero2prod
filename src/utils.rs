//! src/utils.rs
use actix_web::http::header::LOCATION;
use actix_web::HttpResponse;
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};

pub fn e500<T>(e: T) -> actix_web::Error
where
    T: std::fmt::Debug + std::fmt::Display + 'static,
{
    actix_web::error::ErrorInternalServerError(e)
}

pub fn e400<T: std::fmt::Debug + std::fmt::Display + 'static>(e: T) -> actix_web::Error {
    actix_web::error::ErrorBadRequest(e)
}
pub fn see_other(location: &str) -> HttpResponse {
    HttpResponse::SeeOther()
        .insert_header((LOCATION, location))
        .finish()
}

/// 25 字符、大小写敏感的随机 token（确认订阅和退订共用，强度只改一处）。
///
/// 唯一性靠概率而非密码学：25 个 base62 ≈ 149 bit，碰撞可忽略。
/// 真正的安全边界是"token 只出现在收件人的邮箱里"。
pub fn generate_token() -> String {
    let mut rng = thread_rng();
    std::iter::repeat_with(|| rng.sample(Alphanumeric))
        .map(char::from)
        .take(25)
        .collect()
}

/// 把错误链展开成多行，给 thiserror 生成的 Debug 用。
///
/// 价值在于：thiserror 的 Display 只给最外层那句话，真正的数据库错误
/// （撞了哪个约束、哪一列）在 source 里。不展开的话日志里只有
/// "Failed to store the token"，排查时无从下手。
pub fn error_chain_fmt(
    e: &impl std::error::Error,
    f: &mut std::fmt::Formatter<'_>,
) -> std::fmt::Result {
    writeln!(f, "{}\n", e)?;
    let mut current = e.source();
    while let Some(cause) = current {
        writeln!(f, "Caused by:\n\t{}", cause)?;
        current = cause.source();
    }
    Ok(())
}
