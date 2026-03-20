CREATE TABLE a2a_api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id TEXT NOT NULL,
    key_hash TEXT NOT NULL,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    UNIQUE(key_hash)
);

CREATE INDEX idx_a2a_api_keys_customer ON a2a_api_keys(customer_id);
CREATE INDEX idx_a2a_api_keys_hash ON a2a_api_keys(key_hash);
