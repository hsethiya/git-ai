use serde::{Deserialize, Serialize};

use crate::{
    authorship::{
        transcript::AiTranscript,
        working_log::{AgentId, CheckpointKind},
    },
    commands::checkpoint_agent::agent_presets::{AgentCheckpointPreset, AgentRunResult},
};

pub struct AgentV1Preset;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AgentV1Input {
    Human {
        repo_working_dir: String,
        will_edit_filepaths: Option<Vec<String>>,
    },
    AiAgent {
        repo_working_dir: String,
        edited_filepaths: Option<Vec<String>>,
        transcript: AiTranscript,
        agent_name: String,
        model: String,
        conversation_id: String,
    },
    // AiTab
}

impl AgentCheckpointPreset for AgentV1Preset {
    fn run(
        &self,
        flags: super::agent_presets::AgentCheckpointFlags,
    ) -> Result<super::agent_presets::AgentRunResult, crate::error::GitAiError> {
        // Parse hook_input as AgentV1Input, error if it's not valid
        let hook_input_json = flags.hook_input.ok_or_else(|| {
            crate::error::GitAiError::PresetError(
                "--hook-input is required for AgentV1 preset".to_string(),
            )
        })?;

        let agent_v1_input: AgentV1Input = serde_json::from_str(&hook_input_json).map_err(|e| {
            crate::error::GitAiError::PresetError(format!(
                "Invalid AgentV1Input JSON. Format is documented here: https://github.com/acunniffe/git-ai/blob/main/docs/add-your-agent.mdx: \n\n Error: {}",
                e
            ))
        })?;

        match agent_v1_input {
            AgentV1Input::Human {
                repo_working_dir,
                will_edit_filepaths,
            } => Ok(AgentRunResult {
                agent_id: AgentId {
                    tool: "human".to_string(),
                    id: "human".to_string(),
                    model: "human".to_string(),
                },
                will_edit_filepaths: will_edit_filepaths,
                checkpoint_kind: CheckpointKind::Human,
                transcript: None,
                repo_working_dir: Some(repo_working_dir),
                edited_filepaths: None,
            }),
            AgentV1Input::AiAgent {
                edited_filepaths,
                transcript,
                agent_name,
                model,
                conversation_id,
                repo_working_dir,
            } => Ok(AgentRunResult {
                agent_id: AgentId {
                    tool: agent_name,
                    id: conversation_id,
                    model,
                },
                repo_working_dir: Some(repo_working_dir),
                transcript: Some(transcript),
                checkpoint_kind: CheckpointKind::AiAgent,
                edited_filepaths: edited_filepaths,
                will_edit_filepaths: None,
            }),
        }
    }
}
