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
    tenant_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    status TEXT NOT NULL,
    action TEXT,
    decided_by TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, message_id)
);

CREATE TABLE decision_scores (
    tenant_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    category TEXT NOT NULL,
    score DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (tenant_id, message_id, category),
    FOREIGN KEY (tenant_id, message_id)
        REFERENCES decisions (tenant_id, message_id)
);

CREATE TABLE webhook_endpoints (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    url TEXT NOT NULL,
    secret TEXT NOT NULL
);

CREATE INDEX webhook_endpoints_tenant_idx ON webhook_endpoints (tenant_id);
