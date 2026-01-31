-- no-transaction
CREATE UNIQUE INDEX CONCURRENTLY IF NOT EXISTS uniq_active_nonce
ON nonce.nonce_assignments (chain_id, from_address, nonce)
WHERE state IN ('reserved', 'finalized');
