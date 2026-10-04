-- Archived reports leave the owner's working list but stay readable and keep their share link.
ALTER TABLE reports ADD COLUMN archived_at INTEGER;
