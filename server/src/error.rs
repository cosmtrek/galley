use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("unauthorized")]
    Unauthorized,
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Internal(String),
}

pub type AppResult<T> = Result<T, AppError>;

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::NotFound,
            e => AppError::Internal(format!("database: {e}")),
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Internal(format!("json: {e}"))
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Internal(format!("io: {e}"))
    }
}

impl From<crate::domain::TransitionError> for AppError {
    fn from(e: crate::domain::TransitionError) -> Self {
        // The English detail stays in parentheses so agents and logs can still tell which transition failed.
        match e {
            crate::domain::TransitionError::Forbidden { .. } => AppError::Forbidden(format!("没有权限执行这个操作（{e}）")),
            crate::domain::TransitionError::Invalid { .. } => {
                AppError::Conflict(format!("当前状态不允许这个操作，可能已被处理，请刷新页面后重试（{e}）"))
            }
        }
    }
}

impl From<crate::anchor::AnchorError> for AppError {
    fn from(e: crate::anchor::AnchorError) -> Self {
        AppError::BadRequest(format!("评论位置无效，请刷新页面后重新选择（{e}）"))
    }
}

impl AppError {
    /// The message safe to show a client. Internal details are logged instead of returned.
    pub fn client_message(&self) -> String {
        match self {
            AppError::Internal(m) => {
                eprintln!("internal error: {m}");
                "internal error".into()
            }
            e => e.to_string(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(json!({ "error": self.client_message() }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_details_stay_on_the_server() {
        let e = AppError::from(rusqlite::Error::InvalidQuery);
        assert_eq!(e.client_message(), "internal error");
        assert_eq!(AppError::BadRequest("x".into()).client_message(), "x");
    }
}
