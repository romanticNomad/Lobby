CREATE SCHEMA IF NOT EXISTS sign;

CREATE TYPE sign.sign_state AS ENUM (
    'unsigned',
    'signed',
    'failed'
);

CREATE TABLE sign.sign_requests (
    execution_id   BYTEA PRIMARY KEY,
    key_id         TEXT NOT NULL,
    from_address   BYTEA NOT NULL,
    chain_id       BIGINT NOT NULL,
    raw_tx_hash    BYTEA NOT NULL,
    state          sign.sign_state NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_sign_by_sender
ON sign.sign_requests (chain_id, from_address);

CREATE INDEX idx_sign_by_state
ON sign.sign_requests (state);

-- may never be used ...
CREATE INDEX idx_sign_by_raw_tx_hash
ON sign.sign_requests (raw_tx_hash);

-- Ensure updated_at is always correct
CREATE OR REPLACE FUNCTION sign.touch_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_sign_touch_updated_at
BEFORE UPDATE ON sign.sign_requests
FOR EACH ROW
EXECUTE FUNCTION sign.touch_updated_at();
