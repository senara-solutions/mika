use axum::{
    Json,
    extract::{FromRequest, Request, rejection::JsonRejection},
    http::StatusCode,
    response::IntoResponse,
};
use serde::de::DeserializeOwned;

/// Custom JSON extractor that returns JSON error bodies on deserialization failure.
///
/// Axum's default `Json` extractor returns plain-text error messages on invalid input.
/// This wrapper catches `JsonRejection` and converts it to a structured JSON response
/// so all error responses from the API are consistently JSON.
pub struct JsonBody<T>(pub T);

impl<S, T> FromRequest<S> for JsonBody<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = JsonBodyRejection;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(JsonBody(value)),
            Err(rejection) => Err(JsonBodyRejection(rejection)),
        }
    }
}

/// Rejection type that wraps `JsonRejection` and renders as a JSON error body.
pub struct JsonBodyRejection(JsonRejection);

impl IntoResponse for JsonBodyRejection {
    fn into_response(self) -> axum::response::Response {
        let status = match &self.0 {
            JsonRejection::JsonDataError(_) => StatusCode::UNPROCESSABLE_ENTITY,
            JsonRejection::JsonSyntaxError(_) => StatusCode::BAD_REQUEST,
            JsonRejection::MissingJsonContentType(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            _ => StatusCode::BAD_REQUEST,
        };

        let body = serde_json::json!({
            "error": self.0.body_text(),
        });

        (status, Json(body)).into_response()
    }
}
