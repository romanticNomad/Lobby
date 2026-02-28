CREATE SCHEMA IF NOT EXISTS sign;

CREATE TYPE sign.sign_state AS ENUM (
    'reserved',
    'signed',
    'failed'
);

CREATE TABLE sign.sign_requests (
    execution_id   BYTEA NOT NULL,
    revision       BIGINT NOT NULL,
    chain_id       BIGINT NOT NULL,
    from_address   BYTEA NOT NULL,
    state          sign.sign_state NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (execution_id, revision)
);

-- Only active row per execution_id
CREATE UNIQUE INDEX uniq_active_reservation
ON sign.sign_requests (execution_id)
WHERE state IN ('reserved', 'signed');

-- Lookup for latest revision row per execution_id
CREATE INDEX idx_sign_latest_revision
ON sign.sign_requests (execution_id, revision DESC);

-- In case of admin lookups
CREATE INDEX idx_sign_by_state
ON sign.sign_requests (state);

CREATE INDEX idx_broadcast_by_chain
ON sign.sign_requests (chain_id);

-- Ensure updated_at is always called on state change
CREATE OR REPLACE FUNCTION sign.touch_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.state IS DISTINCT FROM OLD.state THEN
        NEW.updated_at = now();
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_sign_touch_updated_at
BEFORE UPDATE ON sign.sign_requests
FOR EACH ROW
EXECUTE FUNCTION sign.touch_updated_at();
