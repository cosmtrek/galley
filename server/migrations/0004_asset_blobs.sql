-- Uploaded files are stored by content hash and never overwritten, so a published version keeps
-- the images it was rendered with. `assets` only maps a name to the latest uploaded content.
CREATE TABLE asset_blobs (
    report_id TEXT NOT NULL REFERENCES reports(id),
    sha256 TEXT NOT NULL,
    mime TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (report_id, sha256)
);
INSERT OR IGNORE INTO asset_blobs (report_id, sha256, mime, created_at)
    SELECT report_id, sha256, mime, created_at FROM assets;
