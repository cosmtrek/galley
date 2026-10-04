-- Comments are a single free-form sentence; intent and reach are written in the body.
ALTER TABLE comments DROP COLUMN kind;
ALTER TABLE comments DROP COLUMN scope;
