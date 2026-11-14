CREATE TABLE messages (
    tenant_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    room_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    text TEXT NOT NULL,
    ts BIGINT NOT NULL,
    reply_to TEXT,
    meta JSONB NOT NULL DEFAULT '{}',
    PRIMARY KEY (tenant_id, message_id)
);

CREATE INDEX messages_tenant_room_ts_idx ON messages (tenant_id, room_id, ts DESC);

CREATE TABLE decisions (
    tenant_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    room_id TEXT NOT NULL,
    status TEXT NOT NULL,
    action TEXT,
    categories JSONB,
    triggered JSONB,
    decided_by TEXT,
    latency_ms BIGINT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, message_id)
);

CREATE INDEX decisions_tenant_room_updated_idx ON decisions (tenant_id, room_id, updated_at);

CREATE TABLE webhook_endpoints (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    url TEXT NOT NULL,
    secret TEXT NOT NULL
);

CREATE INDEX webhook_endpoints_tenant_idx ON webhook_endpoints (tenant_id);
