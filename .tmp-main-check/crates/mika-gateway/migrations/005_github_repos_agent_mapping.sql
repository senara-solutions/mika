-- Add per-repo agent name overrides for multi-tenant webhook routing.
-- Keys are default agent names from route_event() (e.g. "mika-dev"),
-- values are the customer's replacement agent names (e.g. "acme-dev").
-- Empty {} means use defaults.
ALTER TABLE github_repos ADD COLUMN agent_mapping JSONB NOT NULL DEFAULT '{}';
