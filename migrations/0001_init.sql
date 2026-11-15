CREATE TABLE messages (
    tenant_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    room_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    text TEXT NOT NULL,
    timestamp BIGINT NOT NULL,
    reply_to TEXT,
    meta JSONB NOT NULL DEFAULT '{}',
    PRIMARY KEY (tenant_id, message_id)
);

CREATE INDEX messages_tenant_room_timestamp_idx ON messages (tenant_id, room_id, timestamp DESC);

CREATE TABLE decisions (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    status TEXT NOT NULL,
    action TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (tenant_id, message_id)
        REFERENCES messages (tenant_id, message_id)
);

CREATE INDEX decisions_message_idx ON decisions (tenant_id, message_id, id DESC);

CREATE TABLE decision_scores (
    decision_id BIGINT NOT NULL,
    category TEXT NOT NULL,
    score DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (decision_id, category),
    FOREIGN KEY (decision_id) REFERENCES decisions (id)
);

CREATE TABLE webhook_endpoints (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    url TEXT NOT NULL,
    secret TEXT NOT NULL
);

CREATE INDEX webhook_endpoints_tenant_idx ON webhook_endpoints (tenant_id);
