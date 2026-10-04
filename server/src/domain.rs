//! Round and comment state machines. Every status change goes through these functions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentStatus {
    /// 未提交
    Draft,
    /// 待处理
    Open,
    /// 待澄清
    Clarify,
    /// 待验证
    Verify,
    /// 已解决
    Resolved,
    /// 已失效
    Orphaned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentAction {
    Edit,
    Delete,
    Submit,
    /// Owner adds a message (follow-up or answer to a clarification).
    OwnerMessage,
    /// Agent changed the report for this comment.
    AgentChanged,
    /// Agent answered without changing (still needs my verification).
    AgentAnswered,
    /// Agent has a question or disagrees.
    AgentClarify,
    Resolve,
    Reopen,
    /// System: anchored content was deleted.
    Orphan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundStatus {
    Submitted,
    Processing,
    Verifying,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundAction {
    Claim { lease_expired: bool },
    Result,
    /// System: no comment of the round is waiting for verification any more.
    Complete,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TransitionError {
    #[error("{role:?} may not {action} a comment in status {from}")]
    Forbidden { role: Role, action: String, from: String },
    #[error("cannot {action} from status {from}")]
    Invalid { action: String, from: String },
}

macro_rules! str_enum {
    ($t:ty { $($v:ident => $s:literal),* $(,)? }) => {
        impl $t {
            pub fn as_str(&self) -> &'static str { match self { $(Self::$v => $s),* } }
            pub fn parse(s: &str) -> Option<Self> { match s { $($s => Some(Self::$v),)* _ => None } }
        }
    };
}

str_enum!(CommentStatus {
    Draft => "draft", Open => "open", Clarify => "clarify", Verify => "verify",
    Resolved => "resolved", Orphaned => "orphaned",
});
str_enum!(RoundStatus {
    Submitted => "submitted", Processing => "processing", Verifying => "verifying", Done => "done",
});

impl CommentStatus {
    pub fn is_unresolved(&self) -> bool {
        !matches!(self, CommentStatus::Resolved)
    }
}

impl RoundStatus {
    pub fn is_active(&self) -> bool {
        !matches!(self, RoundStatus::Done)
    }
}

fn action_name(a: CommentAction) -> String {
    format!("{a:?}").to_lowercase()
}

/// Returns the next status, or `None` when the comment is deleted.
pub fn comment_transition(
    from: CommentStatus,
    action: CommentAction,
    role: Role,
) -> Result<Option<CommentStatus>, TransitionError> {
    use CommentAction as A;
    use CommentStatus as S;

    let agent_action = matches!(action, A::AgentChanged | A::AgentAnswered | A::AgentClarify);
    let system_action = matches!(action, A::Orphan);
    let allowed_role = if agent_action { Role::Agent } else { Role::Owner };
    if !system_action && role != allowed_role {
        return Err(TransitionError::Forbidden {
            role,
            action: action_name(action),
            from: from.as_str().into(),
        });
    }

    let next = match (from, action) {
        (S::Draft, A::Edit) => Some(S::Draft),
        (S::Draft, A::Delete) => None,
        (S::Draft, A::Submit) => Some(S::Open),
        (S::Draft, A::Orphan) => Some(S::Draft),

        (S::Open, A::OwnerMessage) => Some(S::Open),
        (S::Open, A::AgentChanged | A::AgentAnswered) => Some(S::Verify),
        (S::Open, A::AgentClarify) => Some(S::Clarify),
        (S::Open, A::Orphan) => Some(S::Orphaned),
        // Already-open comments are carried into the next submitted round.
        (S::Open, A::Submit) => Some(S::Open),

        (S::Clarify, A::OwnerMessage) => Some(S::Open),
        (S::Clarify, A::Orphan) => Some(S::Orphaned),
        (S::Clarify, A::Resolve) => Some(S::Resolved),

        (S::Verify, A::Resolve) => Some(S::Resolved),
        (S::Verify, A::Reopen) => Some(S::Open),
        (S::Verify, A::Orphan) => Some(S::Verify),

        (S::Orphaned, A::Resolve) => Some(S::Resolved),
        (S::Orphaned, A::Reopen) => Some(S::Open),
        (S::Orphaned, A::Orphan) => Some(S::Orphaned),

        (S::Resolved, A::Reopen) => Some(S::Open),
        (S::Resolved, A::Orphan) => Some(S::Resolved),

        (from, action) => {
            return Err(TransitionError::Invalid { action: action_name(action), from: from.as_str().into() });
        }
    };
    Ok(next)
}

pub fn round_transition(
    from: RoundStatus,
    action: RoundAction,
    role: Role,
) -> Result<RoundStatus, TransitionError> {
    use RoundStatus as R;
    let name = || match action {
        RoundAction::Claim { .. } => "claim".to_string(),
        RoundAction::Result => "submit a result for".to_string(),
        RoundAction::Complete => "complete".to_string(),
    };
    if matches!(action, RoundAction::Claim { .. } | RoundAction::Result) && role != Role::Agent {
        return Err(TransitionError::Forbidden { role, action: name(), from: from.as_str().into() });
    }
    match (from, action) {
        (R::Submitted, RoundAction::Claim { .. }) => Ok(R::Processing),
        (R::Processing, RoundAction::Claim { lease_expired: true }) => Ok(R::Processing),
        (R::Submitted | R::Processing, RoundAction::Result) => Ok(R::Verifying),
        (R::Verifying, RoundAction::Complete) => Ok(R::Done),
        _ => Err(TransitionError::Invalid { action: name(), from: from.as_str().into() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CommentAction as A;
    use CommentStatus as S;

    const ALL_STATUS: [S; 6] = [S::Draft, S::Open, S::Clarify, S::Verify, S::Resolved, S::Orphaned];
    const ALL_ACTIONS: [A; 11] = [
        A::Edit,
        A::Delete,
        A::Submit,
        A::OwnerMessage,
        A::AgentChanged,
        A::AgentAnswered,
        A::AgentClarify,
        A::Resolve,
        A::Reopen,
        A::Orphan,
        A::Edit,
    ];

    #[test]
    fn agent_can_never_resolve_or_reopen() {
        for s in ALL_STATUS {
            for a in [A::Resolve, A::Reopen, A::Edit, A::Delete, A::Submit, A::OwnerMessage] {
                assert!(
                    matches!(comment_transition(s, a, Role::Agent), Err(TransitionError::Forbidden { .. })),
                    "{s:?} {a:?}"
                );
            }
        }
    }

    #[test]
    fn owner_cannot_impersonate_agent() {
        for s in ALL_STATUS {
            for a in [A::AgentChanged, A::AgentAnswered, A::AgentClarify] {
                assert!(matches!(comment_transition(s, a, Role::Owner), Err(TransitionError::Forbidden { .. })));
            }
        }
    }

    #[test]
    fn legal_comment_transitions() {
        let ok = |s, a, r| comment_transition(s, a, r).unwrap();
        assert_eq!(ok(S::Draft, A::Submit, Role::Owner), Some(S::Open));
        assert_eq!(ok(S::Draft, A::Delete, Role::Owner), None);
        assert_eq!(ok(S::Open, A::AgentChanged, Role::Agent), Some(S::Verify));
        assert_eq!(ok(S::Open, A::AgentAnswered, Role::Agent), Some(S::Verify));
        assert_eq!(ok(S::Open, A::AgentClarify, Role::Agent), Some(S::Clarify));
        assert_eq!(ok(S::Clarify, A::OwnerMessage, Role::Owner), Some(S::Open));
        assert_eq!(ok(S::Verify, A::Resolve, Role::Owner), Some(S::Resolved));
        assert_eq!(ok(S::Verify, A::Reopen, Role::Owner), Some(S::Open));
        assert_eq!(ok(S::Open, A::Orphan, Role::Agent), Some(S::Orphaned));
        assert_eq!(ok(S::Orphaned, A::Resolve, Role::Owner), Some(S::Resolved));
        assert_eq!(ok(S::Resolved, A::Reopen, Role::Owner), Some(S::Open));
    }

    #[test]
    fn illegal_comment_transitions() {
        let bad = [
            (S::Open, A::Edit),
            (S::Open, A::Delete),
            (S::Open, A::Resolve),
            (S::Verify, A::Edit),
            (S::Verify, A::AgentChanged),
            (S::Clarify, A::AgentChanged),
            (S::Resolved, A::Resolve),
            (S::Resolved, A::AgentChanged),
            (S::Draft, A::Resolve),
            (S::Draft, A::AgentChanged),
        ];
        for (s, a) in bad {
            let role = if matches!(a, A::AgentChanged) { Role::Agent } else { Role::Owner };
            assert!(
                matches!(comment_transition(s, a, role), Err(TransitionError::Invalid { .. })),
                "{s:?} {a:?}"
            );
        }
    }

    #[test]
    fn every_pair_is_decided() {
        for s in ALL_STATUS {
            for a in ALL_ACTIONS {
                for r in [Role::Owner, Role::Agent] {
                    let _ = comment_transition(s, a, r);
                }
            }
        }
    }

    #[test]
    fn round_transitions() {
        use RoundStatus as R;
        assert_eq!(round_transition(R::Submitted, RoundAction::Claim { lease_expired: false }, Role::Agent), Ok(R::Processing));
        assert!(round_transition(R::Processing, RoundAction::Claim { lease_expired: false }, Role::Agent).is_err());
        assert_eq!(round_transition(R::Processing, RoundAction::Claim { lease_expired: true }, Role::Agent), Ok(R::Processing));
        assert_eq!(round_transition(R::Processing, RoundAction::Result, Role::Agent), Ok(R::Verifying));
        assert_eq!(round_transition(R::Submitted, RoundAction::Result, Role::Agent), Ok(R::Verifying));
        assert!(round_transition(R::Verifying, RoundAction::Result, Role::Agent).is_err());
        assert!(round_transition(R::Submitted, RoundAction::Claim { lease_expired: false }, Role::Owner).is_err());
        assert!(round_transition(R::Processing, RoundAction::Result, Role::Owner).is_err());
        assert_eq!(round_transition(R::Verifying, RoundAction::Complete, Role::Owner), Ok(R::Done));
        assert!(round_transition(R::Done, RoundAction::Complete, Role::Owner).is_err());
    }
}
