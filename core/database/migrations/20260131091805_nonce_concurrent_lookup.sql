-- no-transaction
CREATE INDEX idx_nonce_by_sender
ON nonce.nonce_assignments (chain_id, from_address);
