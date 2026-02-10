CREATE SCHEMA IF NOT EXISTS relay_host;

CREATE TABLE relay_host.transaction_intents (
    execution_id              BYTEA NOT NULL PRIMARY KEY,
    client_id                 BYTEA NOT NULL,
    chain_id                  BIGINT NOT NULL,
    from_address              BYTEA NOT NULL,
    to_address                BYTEA,  -- NULL for contract creation
    value                     NUMERIC(78, 0) NOT NULL,
    data                      BYTEA NOT NULL,
    gas_limit                 NUMERIC(78, 0) NOT NULL,
    max_fee_per_gas           NUMERIC(78, 0) NOT NULL,
    max_priority_fee_per_gas  NUMERIC(78, 0) NOT NULL,
    access_list               JSONB NOT NULL DEFAULT '[]',
    created_at                TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- Indexes for observability
    CONSTRAINT chk_gas_limit_positive CHECK (gas_limit > 0),
    CONSTRAINT chk_max_fee_positive CHECK (max_fee_per_gas > 0)
);

CREATE INDEX idx_relay_host_client_created 
ON relay_host.transaction_intents (client_id, created_at DESC);

CREATE INDEX idx_relay_host_chain_created 
ON relay_host.transaction_intents (chain_id, created_at DESC);

CREATE INDEX idx_relay_host_from_address 
ON relay_host.transaction_intents (from_address, created_at DESC);

-- Trigger to prevent updates (write-once audit log)
CREATE OR REPLACE FUNCTION relay_host.prevent_updates()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'transaction_intents is immutable';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_prevent_intent_updates
BEFORE UPDATE ON relay_host.transaction_intents
FOR EACH ROW
EXECUTE FUNCTION relay_host.prevent_updates();