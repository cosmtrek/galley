-- Who pushed each version, so a note like "回退到 v1" written by the agent can't pass as the owner's.
-- Round results are the agent's; other existing versions stay unknown (NULL).
ALTER TABLE versions ADD COLUMN author TEXT;
UPDATE versions SET author = 'agent' WHERE round_id IS NOT NULL;
