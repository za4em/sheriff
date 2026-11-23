ALTER TABLE decisions ADD COLUMN group_id TEXT;
UPDATE decisions SET group_id = id::text WHERE group_id IS NULL;
ALTER TABLE decisions ALTER COLUMN group_id SET NOT NULL;
ALTER TABLE decisions DROP COLUMN context;
CREATE INDEX decisions_group_idx ON decisions (group_id);

CREATE TABLE score_windows (
    group_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    message_ids BIGINT[] NOT NULL,
    PRIMARY KEY (group_id, position),
    CHECK (position >= 0),
    CHECK (cardinality(message_ids) > 0)
);
