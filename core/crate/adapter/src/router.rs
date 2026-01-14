use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
use std::sync::Arc;

use kernel::adapter::{
    IntentSink, {Intent, IntentError, IntentResult},
};
