-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_sign_exec_rev_state
ON sign.sign_requests (execution_id, revision, state);
