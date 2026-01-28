use sqlx::PgPool;
use tokio::sync::mpsc;

use crate::sign::SignCommand;

pub struct SignEngine {
    db: PgPool,
    rx: mpsc::Receiver<SignCommand>,
}
