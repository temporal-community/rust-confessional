use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Concurrency,
    Ownership,
    ErrorHandling,
    Unsafe,
    Automation,
    Data,
    Testing,
    Other,
}

impl Category {
    pub const ALL: [Self; 8] = [
        Self::Concurrency,
        Self::Ownership,
        Self::ErrorHandling,
        Self::Unsafe,
        Self::Automation,
        Self::Data,
        Self::Testing,
        Self::Other,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Concurrency => "concurrency",
            Self::Ownership => "ownership",
            Self::ErrorHandling => "error_handling",
            Self::Unsafe => "unsafe",
            Self::Automation => "automation",
            Self::Data => "data",
            Self::Testing => "testing",
            Self::Other => "other",
        }
    }
}

/// Which agent shape a confession's Workflow runs: the fixed linear pipeline or
/// the autonomous decide/act loop. Defaults to `Linear` so existing submissions
/// (and replayed histories missing the field) keep the original behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    #[default]
    Linear,
    Autonomous,
}

/// A capability the autonomous loop can invoke as a research step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Skill {
    RemedyLookup,
    DocLookup,
    SelfCritique,
}

/// One decision the autonomous agent makes on each turn of its loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AgentStep {
    Lookup { skill: Skill, query: String },
    Compose,
    Revise { reason: String },
    Finish,
}

impl AgentStep {
    /// A short, human-readable label for the dashboard's autonomous step trace.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Lookup {
                skill: Skill::RemedyLookup,
                ..
            } => "Looked up an approved remedy",
            Self::Lookup {
                skill: Skill::DocLookup,
                ..
            } => "Consulted the docs",
            Self::Lookup {
                skill: Skill::SelfCritique,
                ..
            } => "Self-critiqued the draft",
            Self::Compose => "Composed a draft",
            Self::Revise { .. } => "Revised the draft",
            Self::Finish => "Finished",
        }
    }
}

/// A result the agent gathered from running a `Skill`, folded into later steps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub skill: Skill,
    pub summary: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmissionInput {
    pub id: String,
    pub session_id: String,
    pub text: String,
    pub created_at: DateTime<Utc>,
    pub hold_before_reply: bool,
    #[serde(default)]
    pub agent_mode: AgentMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPlan {
    pub category: Category,
    pub needs_lookup: bool,
    pub search_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remedy {
    pub category: Category,
    pub guidance: String,
    pub suggested_tools: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwardScores {
    pub most_cursed: u8,
    pub most_relatable: u8,
    pub most_needlessly_rewritten: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Judgment {
    #[serde(default = "default_display_confession")]
    pub display_confession: String,
    pub category: Category,
    pub judgment: String,
    pub severity: u8,
    pub prescription: String,
    pub suggested_tools: Vec<String>,
    pub sentence: String,
    /// A short, playful assignment ("Write a loop that prints foo then bar").
    #[serde(default)]
    pub penance: String,
    /// The single repeatable line the dashboard renders looped `severity` times.
    #[serde(default)]
    pub penance_line: String,
    pub award_scores: AwardScores,
}

fn default_display_confession() -> String {
    "A programming confession.".to_owned()
}

impl Judgment {
    pub fn validate(&self) -> anyhow::Result<()> {
        const MAX_DISPLAY_CHARS: usize = 180;
        const MAX_SHORT_FIELD_CHARS: usize = 280;

        anyhow::ensure!(
            (1..=5).contains(&self.severity),
            "severity must be between 1 and 5"
        );
        anyhow::ensure!(
            self.award_scores.most_cursed <= 100,
            "most_cursed must be <= 100"
        );
        anyhow::ensure!(
            self.award_scores.most_relatable <= 100,
            "most_relatable must be <= 100"
        );
        anyhow::ensure!(
            self.award_scores.most_needlessly_rewritten <= 100,
            "most_needlessly_rewritten must be <= 100"
        );
        anyhow::ensure!(
            !self.display_confession.trim().is_empty(),
            "display_confession cannot be empty"
        );
        anyhow::ensure!(
            self.display_confession.chars().count() <= MAX_DISPLAY_CHARS,
            "display_confession is too long"
        );
        anyhow::ensure!(!self.judgment.trim().is_empty(), "judgment cannot be empty");
        anyhow::ensure!(
            self.judgment.chars().count() <= MAX_SHORT_FIELD_CHARS,
            "judgment is too long"
        );
        anyhow::ensure!(
            !self.prescription.trim().is_empty(),
            "prescription cannot be empty"
        );
        anyhow::ensure!(
            self.prescription.chars().count() <= MAX_SHORT_FIELD_CHARS,
            "prescription is too long"
        );
        anyhow::ensure!(!self.sentence.trim().is_empty(), "sentence cannot be empty");
        anyhow::ensure!(
            self.sentence.chars().count() <= MAX_SHORT_FIELD_CHARS,
            "sentence is too long"
        );
        anyhow::ensure!(!self.penance.trim().is_empty(), "penance cannot be empty");
        anyhow::ensure!(
            self.penance.chars().count() <= MAX_SHORT_FIELD_CHARS,
            "penance is too long"
        );
        anyhow::ensure!(
            !self.penance_line.trim().is_empty(),
            "penance_line cannot be empty"
        );
        anyhow::ensure!(
            self.penance_line.chars().count() <= 48,
            "penance_line is too long"
        );
        anyhow::ensure!(
            !self.penance_line.contains(['\n', '\r']),
            "penance_line must be a single line"
        );
        anyhow::ensure!(
            (1..=5).contains(&self.suggested_tools.len()),
            "suggested_tools must contain between one and five items"
        );
        anyhow::ensure!(
            self.suggested_tools
                .iter()
                .all(|tool| !tool.trim().is_empty() && tool.chars().count() <= 64),
            "suggested_tools contains an invalid item"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionStatus {
    #[default]
    Received,
    Judging,
    Researching,
    Composing,
    ReplyPending,
    Sending,
    Sent,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageSubmission {
    pub id: String,
    pub workflow_id: String,
    pub session_id: String,
    pub text: String,
    pub status: SubmissionStatus,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Category>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judgment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prescription: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sentence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub penance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub penance_line: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub penance_reps: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub award_scores: Option<AwardScores>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Human-readable trace of the autonomous loop's steps. Empty (and omitted
    /// from the projection) for linear confessions, so their rows stay identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_steps: Vec<String>,
}

impl StageSubmission {
    pub fn received(input: &SubmissionInput, workflow_id: String) -> Self {
        Self {
            id: input.id.clone(),
            workflow_id,
            session_id: input.session_id.clone(),
            // Raw audience input is deliberately kept out of the public projection. The
            // structured judgment supplies a stage-safe display version a moment later.
            text: "Confession received — Ferris is reviewing it…".to_owned(),
            status: SubmissionStatus::Received,
            created_at: input.created_at,
            category: None,
            judgment: None,
            severity: None,
            prescription: None,
            sentence: None,
            penance: None,
            penance_line: None,
            penance_reps: None,
            award_scores: None,
            error: None,
            agent_steps: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageUpdate {
    pub id: String,
    pub session_id: String,
    pub status: SubmissionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judgment: Option<Judgment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The running list of autonomous step labels; empty for linear paths.
    #[serde(default)]
    pub agent_steps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSnapshot {
    pub submission: SubmissionInput,
    pub status: SubmissionStatus,
    pub plan: Option<AgentPlan>,
    pub judgment: Option<Judgment>,
    pub released: bool,
    #[serde(default)]
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub steps: Vec<AgentStep>,
}

/// One confession's state as held inside the aggregate `SessionWorkflow`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfession {
    pub submission: SubmissionInput,
    pub status: SubmissionStatus,
    pub plan: Option<AgentPlan>,
    pub judgment: Option<Judgment>,
    /// Per-confession release flag, mirroring `ConfessionWorkflow`: it starts as
    /// `!hold_before_reply` and the release Signal frees the held ones.
    pub released: bool,
    #[serde(default)]
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub steps: Vec<AgentStep>,
}

/// The aggregate `SessionWorkflow`'s durable state, returned by its query. This
/// is the "one durable object holding the whole board" that `state_mut` builds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub session_id: String,
    pub confessions: Vec<SessionConfession>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReleaseInput {
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Awards {
    pub most_cursed: Option<String>,
    pub most_relatable: Option<String>,
    pub most_needlessly_rewritten: Option<String>,
}

/// How Stage turns confessions into Workflows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowMode {
    /// Production shape: one `ConfessionWorkflow` per confession.
    #[default]
    PerConfession,
    /// Demo shape: one aggregate `SessionWorkflow` for the whole session.
    Session,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicStageState {
    pub worker_online: bool,
    pub temporal_connected: bool,
    pub model_mode: String,
    pub held: bool,
    pub show_raw_confessions: bool,
    pub workflow_mode: WorkflowMode,
    pub agent_mode: AgentMode,
    pub submissions: Vec<StageSubmission>,
    pub awards: Awards,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_uses_stable_wire_names() {
        assert_eq!(
            serde_json::to_string(&Category::ErrorHandling).unwrap(),
            "\"error_handling\""
        );
    }

    #[test]
    fn agent_step_labels_are_stable() {
        // The dashboard renders these labels as the autonomous step trace, so
        // pin every variant's wording.
        assert_eq!(
            AgentStep::Lookup {
                skill: Skill::RemedyLookup,
                query: String::new(),
            }
            .label(),
            "Looked up an approved remedy"
        );
        assert_eq!(
            AgentStep::Lookup {
                skill: Skill::DocLookup,
                query: String::new(),
            }
            .label(),
            "Consulted the docs"
        );
        assert_eq!(
            AgentStep::Lookup {
                skill: Skill::SelfCritique,
                query: String::new(),
            }
            .label(),
            "Self-critiqued the draft"
        );
        assert_eq!(AgentStep::Compose.label(), "Composed a draft");
        assert_eq!(
            AgentStep::Revise {
                reason: String::new(),
            }
            .label(),
            "Revised the draft"
        );
        assert_eq!(AgentStep::Finish.label(), "Finished");
    }

    #[test]
    fn judgment_rejects_invalid_severity() {
        let judgment = Judgment {
            display_confession: "I trusted an undocumented invariant.".into(),
            category: Category::Other,
            judgment: "Questionable.".into(),
            severity: 9,
            prescription: "Use a type.".into(),
            suggested_tools: vec![],
            sentence: "Write a test.".into(),
            penance: "Write a loop that prints foo then bar.".into(),
            penance_line: "foo bar".into(),
            award_scores: AwardScores::default(),
        };
        assert!(judgment.validate().is_err());
    }
}
