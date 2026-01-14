use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
use std::sync::Arc;

use kernel::adapter::{
    traits::IntentSink,
    types::{Intent, IntentError, IntentResult},
};
