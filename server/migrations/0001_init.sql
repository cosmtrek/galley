CREATE TABLE reports (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    current_version_id TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE versions (
    id TEXT PRIMARY KEY,
    report_id TEXT NOT NULL REFERENCES reports(id),
    seq INTEGER NOT NULL,
    markdown TEXT NOT NULL,
    html TEXT NOT NULL,
    blocks TEXT NOT NULL,
    block_diff TEXT,
    round_id TEXT,
    note TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    UNIQUE (report_id, seq)
);

CREATE TABLE rounds (
    id TEXT PRIMARY KEY,
    report_id TEXT NOT NULL REFERENCES reports(id),
    seq INTEGER NOT NULL,
    status TEXT NOT NULL,
    base_version_id TEXT NOT NULL,
    result_version_id TEXT,
    summary TEXT NOT NULL DEFAULT '',
    extra_changes TEXT NOT NULL DEFAULT '[]',
    submitted_at INTEGER NOT NULL,
    claimed_at INTEGER,
    completed_at INTEGER,
    UNIQUE (report_id, seq)
);

CREATE TABLE comments (
    id TEXT PRIMARY KEY,
    report_id TEXT NOT NULL REFERENCES reports(id),
    round_id TEXT,
    kind TEXT NOT NULL,
    scope TEXT NOT NULL,
    status TEXT NOT NULL,
    body TEXT NOT NULL,
    created_version_id TEXT NOT NULL,
    resolved_version_id TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX comments_report ON comments(report_id, status);

CREATE TABLE messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    comment_id TEXT NOT NULL REFERENCES comments(id) ON DELETE CASCADE,
    author TEXT NOT NULL,
    action TEXT,
    body TEXT NOT NULL,
    round_id TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX messages_comment ON messages(comment_id);

CREATE TABLE comment_anchors (
    comment_id TEXT NOT NULL REFERENCES comments(id) ON DELETE CASCADE,
    version_id TEXT NOT NULL REFERENCES versions(id),
    anchor TEXT NOT NULL,
    state TEXT NOT NULL,
    PRIMARY KEY (comment_id, version_id)
);

CREATE TABLE publications (
    id TEXT PRIMARY KEY,
    report_id TEXT NOT NULL REFERENCES reports(id),
    version_id TEXT NOT NULL REFERENCES versions(id),
    token TEXT NOT NULL UNIQUE,
    password_hash TEXT,
    expires_at INTEGER,
    revoked_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE views (
    publication_id TEXT NOT NULL REFERENCES publications(id),
    day TEXT NOT NULL,
    count INTEGER NOT NULL,
    last_viewed_at INTEGER NOT NULL,
    PRIMARY KEY (publication_id, day)
);

CREATE TABLE assets (
    id TEXT PRIMARY KEY,
    report_id TEXT NOT NULL REFERENCES reports(id),
    name TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    mime TEXT NOT NULL,
    path TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (report_id, name)
);

CREATE TABLE sessions (
    token TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL
);
