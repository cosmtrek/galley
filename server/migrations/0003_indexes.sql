-- New versions relocate anchors by looking them up per version; the primary key leads with comment_id.
CREATE INDEX comment_anchors_version ON comment_anchors(version_id);
CREATE INDEX comments_round ON comments(round_id, status);
CREATE INDEX publications_report ON publications(report_id);
