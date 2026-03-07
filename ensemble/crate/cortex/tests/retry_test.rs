use cortex::artifacts::retry::*;

// ============================================================
// testing of the generic retry function:

#[cfg(test)]
mod tests {
    use cortex::artifacts::config::RetryConfig;

    use super::*;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        },
        time::Duration,
    };

    #[derive(Debug)]
    pub enum TestError {
        Failuire,
        First,
    }

    impl std::fmt::Display for TestError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self)
        }
    }

    #[tokio::test]
    async fn retries_correct_number_of_times() {
        let cfg = RetryConfig {
            max_attempts: 2 as u32,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(5),
        };

        let calls = Arc::new(AtomicU32::new(0));
        let calls_clone = Arc::clone(&calls);

        // Always fail — should exhaust all retries.
        let result: Result<(), TestError> = retry_with_backoff(&cfg, "test_stage", || {
            let c = Arc::clone(&calls_clone);
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(RetryDecision::Retry(TestError::Failuire))
            }
        })
        .await;

        assert!(result.is_err());
        // max_attempts = 2 → 3 total calls
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn succeeds_on_second_attempt() {
        let cfg = RetryConfig {
            max_attempts: 2,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(5),
        };

        let calls = Arc::new(AtomicU32::new(0));
        let calls_clone = Arc::clone(&calls);

        let result: Result<u32, TestError> = retry_with_backoff(&cfg, "test_stage", || {
            let c = Arc::clone(&calls_clone);
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    Err(RetryDecision::Retry(TestError::First))
                } else {
                    Ok(42)
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn fail_immediately() {
        let cfg = RetryConfig::default(); // default -> retries for 3 times.

        let calls = Arc::new(AtomicU32::new(0));
        let calls_clone = Arc::clone(&calls);

        let result: Result<(), TestError> = retry_with_backoff(&cfg, "test_stage", || {
            let c = Arc::clone(&calls_clone);
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(RetryDecision::FailImmediately(TestError::Failuire))
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

// ============================================================
