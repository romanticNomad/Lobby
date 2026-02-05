CREATE SCHEMA IF NOT EXISTS nonce;

CREATE TYPE nonce.nonce_state AS ENUM (
    'reserved',
    'finalized',
    'released'
);

CREATE TABLE nonce.nonce_assignments (
    execution_id      BYTEA PRIMARY KEY,
    revision          BIGINT NOT NULL,
    chain_id          BIGINT NOT NULL,
    from_address      BYTEA NOT NULL,
    nonce             BIGINT NOT NULL,
    state             nonce.nonce_state NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Only one ('reserved', 'finalized') state row for the same (chain_id, from_address, nonce) allowed
CREATE UNIQUE INDEX uniq_active_nonce
ON nonce.nonce_assignments (chain_id, from_address, nonce)
WHERE state IN ('reserved', 'finalized');

-- Index for efficient revision lookups
CREATE INDEX idx_nonce_latest_revision 
ON nonce.nonce_assignments (execution_id, revision DESC);

-- Lookup scoped by chain_id, from_address
CREATE INDEX idx_nonce_by_sender
ON nonce.nonce_assignments (chain_id, from_address);

-- In case of admin lookup
CREATE INDEX idx_nonce_by_state
ON nonce.nonce_assignments (state);

-- Ensure updated_at is always called on state change
CREATE OR REPLACE FUNCTION nonce.touch_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.state IS DISTINCT FROM OLD.state THEN
        NEW.updated_at = now();
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_nonce_touch_updated_at
BEFORE UPDATE ON nonce.nonce_assignments
FOR EACH ROW
EXECUTE FUNCTION nonce.touch_updated_at();
