CREATE SCHEMA IF NOT EXISTS broadcast;

CREATE TYPE broadcast.broadcast_state AS ENUM (
    'received',
    'submitted',
    'rejected'
);

CREATE TABLE broadcast.broadcast_requests (
    execution_id     BYTEA NOT NULL,
    revision         BIGINT NOT NULL,
    chain_id         BIGINT NOT NULL,
    from_address     BYTEA NOT NULL,
    tx_hash          BYTEA,
    state            broadcast.broadcast_state NOT NULL,
    rejection_reason TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (execution_id, revision)
);

-- Only one active row per execution_id
CREATE UNIQUE INDEX uniq_active_broadcast
ON broadcast.broadcast_requests(execution_id)
WHERE state IN ('received', 'submitted');

-- fast lookup using execution_id, revision (orderable)
CREATE INDEX idx_broadcast_latest_revision
ON broadcast.broadcast_requests (execution_id, revision DESC);

-- Ensure updated_at is always called on state change
CREATE OR REPLACE FUNCTION broadcast.touch_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.state IS DISTINCT FROM OLD.state THEN
        NEW.updated_at = now();
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_broadcast_touch_updated_at
BEFORE UPDATE ON broadcast.broadcast_requests
FOR EACH ROW
EXECUTE FUNCTION broadcast.touch_updated_at();
