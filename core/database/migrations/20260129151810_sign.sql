CREATE SCHEMA IF NOT EXISTS sign;

CREATE TYPE sign.sign_state AS ENUM (
    'unsigned',
    'signed',
    'failed'
);

CREATE TABLE sign.sign_requests (
    execution_id   BYTEA NOT NULL,
    revision       BIGINT NOT NULL,
    key_id         TEXT NOT NULL,
    chain_id       BIGINT NOT NULL,
    from_address   BYTEA NOT NULL,
    state          sign.sign_state NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (execution_id, revision)
);

-- Lookup of latest revision
CREATE INDEX idx_sign_latest_revision
ON sign.sign_requests (execution_id, revision DESC);

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
