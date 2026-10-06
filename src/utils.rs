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

/// 生成一个 25 字符、大小写敏感的随机 token。
///
/// 放在这里而不是各模块各写一份：确认订阅和退订都要 token，
/// 以后若要加长到 32 位或换成 base64url，只改一处就能保证两条链路强度一致。
/// 分成两份很容易出现"新加的那条链路忘了同步加固"。
///
/// 注意：这是【唯一性靠概率】的随机串，不是密码学签名。
/// 25 个 base62 字符 ≈ 149 bit，碰撞概率可以忽略；
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
/// 原来是照抄书里的实现放在 subscribe/persistence.rs，然后被 login 借走 ——
/// 一个"通用错误格式化"函数放在 subscriptions 的持久层里、由 login 反向 import，
/// 依赖方向是反的。退订同样要用它，正好收拢到 utils。
///
/// Debug 里展开 source 链的价值：thiserror 的 Display 只给最外层那句话，
/// 真正的数据库错误（唯一约束名、哪一列）在 source 里。
/// 不展开的话，日志里只会看到"Failed to store the token"，
/// 完全不知道是撞了哪个约束 —— 排查时最耗时的那一步。
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
