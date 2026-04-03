-- Map GitHub repositories to customers for multi-tenant webhook routing.
-- When a webhook event arrives, the gateway looks up the repository's full_name
-- to determine which customer container to route the event to.
CREATE TABLE IF NOT EXISTS github_repos (
    id SERIAL PRIMARY KEY,
    repo_full_name TEXT NOT NULL,
    customer_id UUID NOT NULL REFERENCES customers(id),
    created_at TIMESTAMPTZ DEFAULT NOW()
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_github_repos_name ON github_repos(repo_full_name);
