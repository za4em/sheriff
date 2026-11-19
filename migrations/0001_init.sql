CREATE TABLE messages (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    tenant_id  TEXT NOT NULL,
    client_message_id TEXT NOT NULL,
    room_id    TEXT NOT NULL,
    user_id    TEXT NOT NULL,
    text       TEXT NOT NULL,
    timestamp  BIGINT NOT NULL,
    reply_to   TEXT,
    meta       JSONB NOT NULL DEFAULT '{}',
    retained   BOOLEAN NOT NULL,
    UNIQUE (id, tenant_id)
);

CREATE INDEX messages_room_idx
    ON messages (tenant_id, room_id, timestamp DESC)
    WHERE retained;

CREATE TABLE decisions (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    tenant_id  TEXT NOT NULL,
    message_id BIGINT NOT NULL UNIQUE,
    status     TEXT NOT NULL,
    action     TEXT,
    error      TEXT,
    context    BIGINT[] NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (message_id, tenant_id) REFERENCES messages (id, tenant_id),
    CHECK (status IN ('pending', 'ready', 'failed')),
    CHECK (action IS NULL OR action IN ('allow', 'hide', 'mute', 'escalate')),
    CHECK (
        (status = 'pending' AND action IS NULL AND error IS NULL)
        OR (status = 'ready' AND action IS NOT NULL AND error IS NULL)
        OR (status = 'failed' AND action IS NULL AND error IS NOT NULL)
    )
);

CREATE TABLE decision_scores (
    decision_id BIGINT NOT NULL REFERENCES decisions (id) ON DELETE CASCADE,
    category    TEXT NOT NULL,
    score       DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (decision_id, category),
    CHECK (score >= 0 AND score <= 1)
);

CREATE TABLE webhook_endpoints (
    id        BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    url       TEXT NOT NULL,
    secret    TEXT NOT NULL
);

CREATE INDEX webhook_endpoints_tenant_idx ON webhook_endpoints (tenant_id);
