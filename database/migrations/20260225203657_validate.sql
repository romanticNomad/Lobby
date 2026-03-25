CREATE SCHEMA IF NOT EXISTS validator;

CREATE TABLE validator.validation_requests (
    execution_id        BYTEA NOT NULL,
    revision            INT NOT NULL,
    chain_id            BIGINT NOT NULL,
    tx_hash             BYTEA NOT NULL,
    state               TEXT NOT NULL CHECK (state IN ('pending', 'included', 'not_included', 'timed_out')),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (execution_id, revision),

    -- Ensure tx_hash is valid (32 bytes for B256)
    CONSTRAINT tx_hash_length CHECK (octet_length(tx_hash) = 32)
);


-- Idempotency check: find active pending validations
CREATE INDEX idx_validation_active
ON validator.validation_requests (execution_id, updated_at)
WHERE state = 'pending';

-- Lookup completed validations for caching
CREATE INDEX idx_validation_outcome
ON validator.validation_requests (execution_id, state);

-- Audit queries: find all validations for a given transaction hash outcome
CREATE INDEX idx_validation_by_tx_hash
ON validator.validation_requests (tx_hash, chain_id);


-- Partial unique index: prevent concurrent pending validations for the same execution_id
CREATE UNIQUE INDEX idx_validation_no_concurrent
ON validator.validation_requests (execution_id)
WHERE state = 'pending';


-- Trigger: auto-update updated_at timestamp
CREATE OR REPLACE FUNCTION validator.update_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_validation_updated_at
BEFORE UPDATE ON validator.validation_requests
FOR EACH ROW
EXECUTE FUNCTION validator.update_updated_at();


-- Comments
COMMENT ON TABLE validator.validation_requests IS
'Tracks the validation status of broadcast transactions. Each execution_id can have multiple revisions as the validation progresses through states: pending → included/not_included.';

COMMENT ON COLUMN validator.validation_requests.state IS
'Current state: pending (polling RPC), included (confirmed on-chain), not_included (timeout/reorg/reverted).';

COMMENT ON COLUMN validator.validation_requests.updated_at IS
'Timestamp of the last state change. Used for 5-minute lease-based idempotency.';
