-- Small instance-wide values (e.g. when an agent last authenticated) that don't belong to any report.
CREATE TABLE kv (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
