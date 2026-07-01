CREATE SCHEMA IF NOT EXISTS nonce;

CREATE TYPE nonce.nonce_state AS ENUM (
    'reserved',
    'finalized',
    'released',
    'consumed'
);

CREATE TABLE nonce.nonce_assignments (
    execution_id      BYTEA NOT NULL,
    revision          BIGINT NOT NULL,
    chain_id          BIGINT NOT NULL,
    from_address      BYTEA NOT NULL,
    nonce             BIGINT NOT NULL,
    state             nonce.nonce_state NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (execution_id, revision)
);

-- Only one ('reserved', 'finalized') state row for the same (chain_id, from_address, nonce) allowed
CREATE UNIQUE INDEX uniq_active_nonce
ON nonce.nonce_assignments (chain_id, from_address, nonce)
WHERE state IN ('reserved', 'finalized');

-- special index for selecting 'released' nonce
CREATE INDEX idx_nonce_released_gap_fill
ON nonce.nonce_assignments (chain_id, from_address, nonce ASC, revision DESC)
WHERE state = 'released';

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
