use std::error::Error;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use tracing::{error, debug};

pub mod ts_float_seconds {
    use chrono::{DateTime, NaiveDateTime, Utc};
    use serde::de;

    pub struct SecondsTimestampVisitor;

    pub fn deserialize<'de, D>(d: D) -> Result<DateTime<Utc>, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        d.deserialize_f64(SecondsTimestampVisitor)
    }

    impl<'de> de::Visitor<'de> for SecondsTimestampVisitor {
        type Value = DateTime<Utc>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a unix timestamp in seconds")
        }

        /// Deserialize a timestamp in seconds since the epoch
        fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            let naive = NaiveDateTime::from_timestamp(value as i64, 0);
            Ok(DateTime::from_utc(naive, Utc))
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum HttpError {
    #[error("not found")]
    NotFound,

    #[error("already exists")]
    AlreadyExists,

    #[error("a database error occurred")]
    Sqlx(#[from] sqlx::Error),

    #[error("internal server error")]
    Other(#[from] Box<dyn Error>),
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let msg = self.to_string();

        match self {
            HttpError::NotFound => (StatusCode::NOT_FOUND, msg).into_response(),
            HttpError::AlreadyExists => (StatusCode::CONFLICT, msg).into_response(),
            HttpError::Sqlx(_) | HttpError::Other(_) => {
                error!(%self, "internal server error");
                debug!(?self, "internal server error");
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}
