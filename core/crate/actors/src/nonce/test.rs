#[cfg(test)]
mod tests {
    use crate::nonce::NonceActor;
    use dotenvy::dotenv;
    use sqlx::PgPool;
    use tokio::sync::mpsc;


    // dummy function to establish db connectivity.
    #[tokio::test]
    async fn nonce_actor_db_smoke() {
        dotenv().ok();

        let pool = PgPool::connect(
            &std::env::var("DATABASE_URL").unwrap()
        )
        .await
        .unwrap();

        let (_tx, rx) = mpsc::channel(8);

        let _actor = NonceActor::new(pool, rx);
    }
}
