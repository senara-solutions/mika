CREATE TABLE outbound_messages (
    telegram_message_id BIGINT NOT NULL,
    chat_id BIGINT NOT NULL,
    agent_name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (telegram_message_id, chat_id)
);

CREATE INDEX idx_outbound_messages_created ON outbound_messages(created_at);
