-- Which changed blocks each edit of a round result touched, keyed by the comment whose reply carried it
-- (see `Attribution` in store.rs). Results sent as whole Markdown leave it empty.
ALTER TABLE rounds ADD COLUMN attribution TEXT NOT NULL DEFAULT '{}';
