CREATE SCHEMA IF NOT EXISTS nonce;

CREATE TYPE nonce.nonce_state AS ENUM (
    'reserved',
    'finalized',
    'released'
);

CREATE TABLE nonce.nonce_assignments (
    execution_id      BYTEA PRIMARY KEY,
    chain_id          BIGINT NOT NULL,
    from_address      BYTEA NOT NULL,
    nonce             BIGINT NOT NULL,
    state             nonce.nonce_state NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Ensures nonce uniqueness during race condition
CREATE UNIQUE INDEX uniq_active_nonce
ON nonce.nonce_assignments (chain_id, from_address, nonce)
WHERE state IN ('reserved', 'finalized');

CREATE INDEX idx_nonce_by_sender
ON nonce.nonce_assignments (chain_id, from_address);

CREATE INDEX idx_nonce_by_state
ON nonce.nonce_assignments (state);

-- Ensure updated_at is always correct
CREATE OR REPLACE FUNCTION nonce.touch_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_touch_updated_at
BEFORE UPDATE ON nonce.nonce_assignments
FOR EACH ROW
EXECUTE FUNCTION nonce.touch_updated_at();
