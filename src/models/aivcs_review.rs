//! AIVCS review projections (issue #148, wave-1 slice 2).
//!
//! These types mirror the SQL projection tables introduced in
//! `migrations/0016_aivcs_review_projections.sql`. They are
//! intentionally separate from the (forthcoming) slice-1
//! `models::aivcs` module: that module owns the AIVCS *event
//! taxonomy*; this one owns the *read-model shape* the UI consumes.
//!
//! Provenance lives in events. Projections live here.
//!
//! No HTTP routes are exposed in this slice — the types are reachable
//! from `db.rs` CRUD helpers only.

use serde::{Deserialize, Serialize};

// ── Enums ───────────────────────────────────────────────────────

/// Lifecycle status of a single review thread.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewThreadStatus {
    /// Newly created or re-opened thread; not yet resolved.
    Open,
    /// Thread has been resolved (see [`ReviewThreadResolution`]).
    Resolved,
}

impl ReviewThreadStatus {
    /// SQL-encoded representation matching the `status` column of `review_thread`.
    pub fn as_sql(&self) -> &'static str {
        match self {
            ReviewThreadStatus::Open => "open",
            ReviewThreadStatus::Resolved => "resolved",
        }
    }

    /// Parse the on-disk SQL form back to a [`ReviewThreadStatus`].
    pub fn from_sql(s: &str) -> Option<Self> {
        match s {
            "open" => Some(ReviewThreadStatus::Open),
            "resolved" => Some(ReviewThreadStatus::Resolved),
            _ => None,
        }
    }
}

/// Comment actor — either a human user or an AI agent. The `id` is
/// the AIVCS-side identifier (e.g. `jane.taylor` for humans,
/// `optimizer-7` for agents) — matching the event-taxonomy actor
/// shape (`human:<id>` / `agent:<id>`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum CommentActor {
    Human(String),
    Agent(String),
}

impl CommentActor {
    /// Render in the on-disk shape used by the `review_comment.actor` column.
    pub fn to_sql_actor(&self) -> String {
        match self {
            CommentActor::Human(id) => format!("human:{id}"),
            CommentActor::Agent(id) => format!("agent:{id}"),
        }
    }

    /// Parse the on-disk `human:<id>` / `agent:<id>` form.
    ///
    /// Returns `None` for any other prefix, an empty id, or an unknown kind.
    pub fn from_sql_actor(s: &str) -> Option<Self> {
        let (kind, id) = s.split_once(':')?;
        if id.is_empty() {
            return None;
        }
        match kind {
            "human" => Some(CommentActor::Human(id.to_owned())),
            "agent" => Some(CommentActor::Agent(id.to_owned())),
            _ => None,
        }
    }
}

/// Why a review thread was closed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// The reported issue was addressed in code.
    Fixed,
    /// The reported issue was acknowledged but will not be acted on.
    WontFix,
    /// The thread duplicates another open or resolved thread.
    Duplicate,
    /// The thread was a discussion that reached a conclusion without code change.
    Discussion,
}

impl Resolution {
    pub fn as_sql(&self) -> &'static str {
        match self {
            Resolution::Fixed => "fixed",
            Resolution::WontFix => "wont_fix",
            Resolution::Duplicate => "duplicate",
            Resolution::Discussion => "discussion",
        }
    }

    pub fn from_sql(s: &str) -> Option<Self> {
        match s {
            "fixed" => Some(Resolution::Fixed),
            "wont_fix" => Some(Resolution::WontFix),
            "duplicate" => Some(Resolution::Duplicate),
            "discussion" => Some(Resolution::Discussion),
            _ => None,
        }
    }
}

/// Side of a diff a file anchor pins to.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnchorSide {
    /// Base side of the diff.
    Left,
    /// Head side of the diff (default for new comments).
    Right,
}

impl AnchorSide {
    pub fn as_sql(&self) -> &'static str {
        match self {
            AnchorSide::Left => "left",
            AnchorSide::Right => "right",
        }
    }

    pub fn from_sql(s: &str) -> Option<Self> {
        match s {
            "left" => Some(AnchorSide::Left),
            "right" => Some(AnchorSide::Right),
            _ => None,
        }
    }
}

// ── Structs ─────────────────────────────────────────────────────

/// One review conversation, attached to a review and (optionally) a change set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewThread {
    pub id: String,
    pub review_id: String,
    pub change_set_id: Option<String>,
    pub status: ReviewThreadStatus,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

/// One comment in a review thread.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewComment {
    pub id: String,
    pub thread_id: String,
    pub actor: CommentActor,
    pub body: String,
    pub parent_comment_id: Option<String>,
    pub created_at: String,
}

/// Resolution metadata for a review thread.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewThreadResolution {
    pub thread_id: String,
    pub resolved_by: String,
    pub resolution: Resolution,
    pub note: Option<String>,
    pub resolved_at: String,
}

/// Error returned when an attempted [`FileAnchor`] has `end_line < start_line`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidLineRange {
    pub start_line: i64,
    pub end_line: i64,
}

impl std::fmt::Display for InvalidLineRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "file_anchor: end_line ({}) must be >= start_line ({})",
            self.end_line, self.start_line
        )
    }
}

impl std::error::Error for InvalidLineRange {}

/// Pins a thread to a file + line range on one side of the diff.
///
/// Construction goes through [`FileAnchor::new`], which rejects
/// `end_line < start_line` at the type level. The struct fields are
/// private so the only way to land an invalid anchor is via direct
/// deserialisation from trusted projection storage (`from_row`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileAnchor {
    pub id: String,
    pub thread_id: String,
    pub file_path: String,
    start_line: i64,
    end_line: i64,
    pub side: AnchorSide,
}

impl FileAnchor {
    /// Construct a new [`FileAnchor`], validating the line range.
    pub fn new(
        id: String,
        thread_id: String,
        file_path: String,
        start_line: i64,
        end_line: i64,
        side: AnchorSide,
    ) -> std::result::Result<Self, InvalidLineRange> {
        if end_line < start_line {
            return Err(InvalidLineRange {
                start_line,
                end_line,
            });
        }
        Ok(Self {
            id,
            thread_id,
            file_path,
            start_line,
            end_line,
            side,
        })
    }

    /// Re-hydrate an anchor from trusted projection storage. The
    /// migration enforces NOT NULL on `start_line` / `end_line`, but
    /// not the ordering — we still validate here so an external
    /// writer cannot smuggle in `end_line < start_line`.
    pub fn from_row(
        id: String,
        thread_id: String,
        file_path: String,
        start_line: i64,
        end_line: i64,
        side: AnchorSide,
    ) -> std::result::Result<Self, InvalidLineRange> {
        Self::new(id, thread_id, file_path, start_line, end_line, side)
    }

    pub fn start_line(&self) -> i64 {
        self.start_line
    }

    pub fn end_line(&self) -> i64 {
        self.end_line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Enum SQL round-trips ────────────────────────────────────

    #[test]
    fn review_thread_status_sql_round_trip() {
        for s in [ReviewThreadStatus::Open, ReviewThreadStatus::Resolved] {
            let sql = s.as_sql();
            let parsed = ReviewThreadStatus::from_sql(sql).expect("known status");
            assert_eq!(parsed, s);
        }
        assert!(ReviewThreadStatus::from_sql("bogus").is_none());
    }

    #[test]
    fn review_thread_status_serde_round_trip() {
        // Both variants must JSON-round-trip in snake_case, since the
        // BFF will read this shape directly off the wire.
        for (s, expected) in [
            (ReviewThreadStatus::Open, "\"open\""),
            (ReviewThreadStatus::Resolved, "\"resolved\""),
        ] {
            let json = serde_json::to_string(&s).unwrap();
            assert_eq!(json, expected);
            let parsed: ReviewThreadStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, s);
        }
    }

    #[test]
    fn resolution_sql_round_trip() {
        for r in [
            Resolution::Fixed,
            Resolution::WontFix,
            Resolution::Duplicate,
            Resolution::Discussion,
        ] {
            assert_eq!(Resolution::from_sql(r.as_sql()), Some(r));
        }
        assert!(Resolution::from_sql("retired").is_none());
    }

    #[test]
    fn anchor_side_sql_round_trip() {
        for s in [AnchorSide::Left, AnchorSide::Right] {
            assert_eq!(AnchorSide::from_sql(s.as_sql()), Some(s));
        }
        assert!(AnchorSide::from_sql("center").is_none());
    }

    // ── CommentActor parsing ────────────────────────────────────

    #[test]
    fn comment_actor_parses_human_jane() {
        let parsed = CommentActor::from_sql_actor("human:jane").expect("parses");
        assert_eq!(parsed, CommentActor::Human("jane".into()));
        // Round-trips through the SQL representation.
        assert_eq!(parsed.to_sql_actor(), "human:jane");
    }

    #[test]
    fn comment_actor_parses_agent_optimizer_7() {
        let parsed = CommentActor::from_sql_actor("agent:optimizer-7").expect("parses");
        assert_eq!(parsed, CommentActor::Agent("optimizer-7".into()));
        assert_eq!(parsed.to_sql_actor(), "agent:optimizer-7");
    }

    #[test]
    fn comment_actor_rejects_malformed_strings() {
        // No colon -> no split possible.
        assert!(CommentActor::from_sql_actor("janedoe").is_none());
        // Unknown kind.
        assert!(CommentActor::from_sql_actor("bot:1").is_none());
        // Empty id.
        assert!(CommentActor::from_sql_actor("human:").is_none());
    }

    #[test]
    fn comment_actor_id_may_contain_colons() {
        // Tenant-qualified agent ids ("agent:tenant-a:optimizer-7") must
        // survive the split — split_once keeps the tail intact.
        let parsed = CommentActor::from_sql_actor("agent:tenant-a:optimizer-7").expect("parses");
        assert_eq!(parsed, CommentActor::Agent("tenant-a:optimizer-7".into()));
    }

    // ── FileAnchor invariants ───────────────────────────────────

    #[test]
    fn file_anchor_accepts_valid_line_ranges() {
        // Single-line anchor.
        let single = FileAnchor::new(
            "fa1".into(),
            "th1".into(),
            "src/lib.rs".into(),
            10,
            10,
            AnchorSide::Right,
        )
        .expect("single-line accepted");
        assert_eq!(single.start_line(), 10);
        assert_eq!(single.end_line(), 10);

        // Multi-line anchor.
        let multi = FileAnchor::new(
            "fa2".into(),
            "th1".into(),
            "src/lib.rs".into(),
            10,
            42,
            AnchorSide::Left,
        )
        .expect("multi-line accepted");
        assert_eq!(multi.start_line(), 10);
        assert_eq!(multi.end_line(), 42);
        assert_eq!(multi.side, AnchorSide::Left);
    }

    #[test]
    fn file_anchor_rejects_inverted_line_ranges() {
        let err = FileAnchor::new(
            "fa3".into(),
            "th1".into(),
            "src/lib.rs".into(),
            42,
            10,
            AnchorSide::Right,
        )
        .expect_err("must reject end_line < start_line");
        assert_eq!(err.start_line, 42);
        assert_eq!(err.end_line, 10);
    }

    #[test]
    fn file_anchor_from_row_validates_too() {
        // Even from "trusted" storage we re-validate, so a malicious
        // direct INSERT cannot smuggle in an inverted range.
        assert!(FileAnchor::from_row(
            "fa4".into(),
            "th1".into(),
            "src/lib.rs".into(),
            100,
            50,
            AnchorSide::Right,
        )
        .is_err());
    }

    // ── Struct serde round-trips ────────────────────────────────

    #[test]
    fn review_thread_round_trip() {
        let t = ReviewThread {
            id: "th1".into(),
            review_id: "rev1".into(),
            change_set_id: Some("chg1".into()),
            status: ReviewThreadStatus::Open,
            created_at: "2026-01-01T00:00:00Z".into(),
            resolved_at: None,
        };
        let json = serde_json::to_string(&t).unwrap();
        let parsed: ReviewThread = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, t);
    }

    #[test]
    fn review_comment_round_trip() {
        let c = ReviewComment {
            id: "c1".into(),
            thread_id: "th1".into(),
            actor: CommentActor::Human("jane".into()),
            body: "looks good".into(),
            parent_comment_id: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&c).unwrap();
        let parsed: ReviewComment = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, c);
    }

    #[test]
    fn review_thread_resolution_round_trip() {
        let r = ReviewThreadResolution {
            thread_id: "th1".into(),
            resolved_by: "human:jane".into(),
            resolution: Resolution::Fixed,
            note: Some("addressed in chg2".into()),
            resolved_at: "2026-01-02T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&r).unwrap();
        let parsed: ReviewThreadResolution = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, r);
    }
}
