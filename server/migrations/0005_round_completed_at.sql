-- `completed_at` used to be written when the agent submitted a result; it now means the round is done.
UPDATE rounds SET completed_at = NULL WHERE status != 'done';
