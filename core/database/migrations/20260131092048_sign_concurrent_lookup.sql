-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_sign_latest_revision
ON sign.sign_requests (execution_id, revision DESC);
