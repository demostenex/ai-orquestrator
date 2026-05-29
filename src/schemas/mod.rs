use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type SchemaResult<T> = Result<T, ValidationError>;

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub description: String,
    pub status: String,
    pub assigned_to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub run_id: Option<String>,
    pub title: String,
    pub status: String,
    pub tasks: Vec<Task>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevResponse {
    pub step_id: String,
    pub summary: String,
    pub files_touched: Vec<String>,
    pub diff: String,
    pub tests_suggested: Vec<String>,
    pub risks: Vec<String>,
}

impl DevResponse {
    pub fn validate(&self) -> SchemaResult<()> {
        if self.step_id.trim().is_empty() {
            return Err(ValidationError::Message("step_id cannot be empty".into()));
        }

        if self.diff.trim().is_empty() {
            return Err(ValidationError::Message("diff cannot be empty".into()));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditResponse {
    pub approved: bool,
    pub score: u8,
    pub problems: Vec<String>,
    pub required_changes: Vec<String>,
    pub blocked_reason: Option<String>,
}

impl AuditResponse {
    pub fn validate(&self) -> SchemaResult<()> {
        if self.score > 100 {
            return Err(ValidationError::Message("score must be between 0 and 100".into()));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HandoffStatus {
    WaitingAudit,
    Approved,
    Rejected,
    ReadyForDev,
    WaitingHuman,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Handoff {
    pub agent: String,
    pub target_agent: String,
    pub step_id: String,
    pub status: HandoffStatus,
    pub summary: String,
    pub decisions: Vec<String>,
    pub files_touched: Vec<String>,
    pub open_questions: Vec<String>,
    pub risks: Vec<String>,
    pub next_action: String,
}

impl Handoff {
    pub fn validate(&self) -> SchemaResult<()> {
        if self.agent.trim().is_empty() {
            return Err(ValidationError::Message("agent cannot be empty".into()));
        }
        if self.target_agent.trim().is_empty() {
            return Err(ValidationError::Message("target_agent cannot be empty".into()));
        }
        if self.step_id.trim().is_empty() {
            return Err(ValidationError::Message("step_id cannot be empty".into()));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CycleStatus {
    Initialized,
    DevDone,
    AuditRequested,
    Approved,
    Rejected,
    Applied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleState {
    pub run_id: String,
    pub step_id: String,
    pub status: CycleStatus,
    pub base_commit: String,
    pub plan_hash: String,
    pub memory_hash: String,
    pub patch_file: Option<String>,
    pub patch_hash: Option<String>,
    pub audit_file: Option<String>,
    pub audit_approved: Option<bool>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl CycleState {
    pub fn validate(&self) -> SchemaResult<()> {
        if self.run_id.trim().is_empty() {
            return Err(ValidationError::Message("run_id cannot be empty".into()));
        }
        if self.step_id.trim().is_empty() {
            return Err(ValidationError::Message("step_id cannot be empty".into()));
        }
        if self.base_commit.trim().is_empty() {
            return Err(ValidationError::Message("base_commit cannot be empty".into()));
        }
        if self.plan_hash.trim().is_empty() {
            return Err(ValidationError::Message("plan_hash cannot be empty".into()));
        }
        if self.memory_hash.trim().is_empty() {
            return Err(ValidationError::Message("memory_hash cannot be empty".into()));
        }

        Ok(())
    }
}
