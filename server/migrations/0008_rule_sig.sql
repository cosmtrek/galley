-- Horizontal rules used to carry their parser event index as a signature, so any edit above a rule
-- broke its alignment. Rules now have no signature; drop the old ones so the next version aligns.
UPDATE versions
SET blocks = json_set(blocks, '$.blocks', json((
    SELECT json_group_array(CASE WHEN json_extract(value, '$.kind') = 'rule' THEN json_remove(value, '$.sig') ELSE json(value) END)
    FROM json_each(versions.blocks, '$.blocks')
)))
WHERE blocks LIKE '%"kind":"rule"%';
