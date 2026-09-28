//! src/utils.rs
use actix_web::HttpResponse;
use actix_web::http::header::LOCATION;

pub fn e500<T>(e: T) -> actix_web::Error
where
    T: std::fmt::Debug + std::fmt::Display + 'static {
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