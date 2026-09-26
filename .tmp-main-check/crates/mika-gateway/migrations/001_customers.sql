CREATE TABLE customers (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    plan TEXT NOT NULL DEFAULT 'standard'
        CHECK (plan IN ('standard', 'premium')),
    status TEXT NOT NULL DEFAULT 'provisioned'
        CHECK (status IN ('provisioned', 'active', 'suspended')),
    telegram_chat_id BIGINT UNIQUE,
    timezone TEXT NOT NULL DEFAULT 'UTC',
    pairing_token TEXT UNIQUE,
    pairing_expires_at TIMESTAMPTZ,
    last_update_id BIGINT NOT NULL DEFAULT 0,
    paired_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_customers_status ON customers(status);
CREATE INDEX idx_customers_pairing_token ON customers(pairing_token) WHERE pairing_token IS NOT NULL;

-- Auto-update updated_at on row change
CREATE OR REPLACE FUNCTION update_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER customers_updated_at
    BEFORE UPDATE ON customers
    FOR EACH ROW EXECUTE FUNCTION update_updated_at();
