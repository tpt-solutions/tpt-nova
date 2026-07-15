//! Optional LLM integration for the "Highlight & Fix" loop.
//!
//! Given a [`nova_overlay::AiFixRequest`] (built from a viewport marquee plus
//! the user's typed instruction) and optional RAG context, an [`LlmClient`]
//! produces an `AgentCommand` batch that is applied to the world through the
//! same bounded [`apply_commands`] path the file control channel uses. Only
//! `AgentCommand`s are ever applied, so the LLM can reposition/spawn entities
//! but can never touch engine internals or run arbitrary code.
//!
//! The offline-default [`HeuristicLlm`] maps natural-language directions to
//! transforms with no network. A real provider is available behind the `llm`
//! feature ([`HttpLlm`]), reading its endpoint/key from the environment.

use crate::{apply_commands, AgentApiError, AgentCommand, EntityRef, World};
use nova_overlay::AiFixRequest;

/// Errors from LLM completion or command parsing.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("llm client error: {0}")]
    Client(String),
    #[error("failed to parse agent commands from LLM output: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("fix request had no target entity")]
    NoEntity,
}

/// A pluggable completion client. The engine only requires text in / text out;
/// how the model is reached (local, hosted, mocked) is the client's concern.
pub trait LlmClient: Send + Sync {
    /// Given a prompt, return the model's raw text response.
    fn complete(&self, prompt: &str) -> Result<String, LlmError>;
}

/// Translate a direction word in `instruction` into a world-space translation
/// delta (1 unit per matched axis). Returns `None` when no direction is present.
fn direction_delta(instruction: &str) -> Option<(f32, f32, f32)> {
    let up = instruction.contains("up");
    let down = instruction.contains("down");
    let left = instruction.contains("left");
    let right = instruction.contains("right");
    if !up && !down && !left && !right {
        return None;
    }
    let mut d = (0.0f32, 0.0f32, 0.0f32);
    if up {
        d.1 += 1.0;
    }
    if down {
        d.1 -= 1.0;
    }
    if right {
        d.0 += 1.0;
    }
    if left {
        d.0 -= 1.0;
    }
    Some(d)
}

/// Find the first telemetry entity id (`e<index>#<generation>`) in `text`.
fn extract_first_entity_id(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'e' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1 && j < bytes.len() && bytes[j] == b'#' {
                let mut k = j + 1;
                while k < bytes.len() && bytes[k].is_ascii_digit() {
                    k += 1;
                }
                if k > j + 1 {
                    return Some(text[i..k].to_string());
                }
            }
        }
        i += 1;
    }
    None
}

/// Map a fix request's typed instruction to an `AgentCommand` batch offline.
/// This is the testable core of the heuristic path and is also used directly by
/// [`apply_fix_heuristic`].
pub fn fix_request_to_commands(req: &AiFixRequest) -> Result<Vec<AgentCommand>, LlmError> {
    let id = req
        .entity_ids
        .first()
        .ok_or(LlmError::NoEntity)?
        .clone();
    match direction_delta(&req.instruction) {
        Some((dx, dy, dz)) => Ok(vec![AgentCommand::SetTransform {
            target: EntityRef::Id(id),
            translation: Some([dx, dy, dz]),
            rotation_euler_xyz: None,
            scale: None,
        }]),
        None => Ok(vec![]),
    }
}

/// An offline stand-in for a real LLM: it parses the prompt the same way a model
/// would be asked to, and emits the `AgentCommand` JSON a real client would
/// return. Lets the full fix loop run with zero network.
pub struct HeuristicLlm;

impl LlmClient for HeuristicLlm {
    fn complete(&self, prompt: &str) -> Result<String, LlmError> {
        let id = extract_first_entity_id(prompt).ok_or(LlmError::NoEntity)?;
        match direction_delta(prompt) {
            Some((dx, dy, dz)) => {
                let cmd = AgentCommand::SetTransform {
                    target: EntityRef::Id(id),
                    translation: Some([dx, dy, dz]),
                    rotation_euler_xyz: None,
                    scale: None,
                };
                Ok(serde_json::to_string(&[cmd])?)
            }
            None => Ok("[]".to_string()),
        }
    }
}

/// A real HTTP LLM client. Reads its endpoint from `NOVA_LLM_ENDPOINT` and an
/// auth token from `NOVA_LLM_KEY` (optional). Returns `None` from
/// [`HttpLlm::from_env`] when no endpoint is configured, so callers can fall
/// back to the heuristic client.
#[cfg(feature = "llm")]
pub struct HttpLlm {
    endpoint: String,
    api_key: String,
    client: reqwest::blocking::Client,
}

#[cfg(feature = "llm")]
impl HttpLlm {
    /// Build a client from the environment, or `None` if `NOVA_LLM_ENDPOINT` is
    /// unset (so the caller can fall back to [`HeuristicLlm`]).
    pub fn from_env() -> Option<Self> {
        let endpoint = std::env::var("NOVA_LLM_ENDPOINT").ok()?;
        let api_key = std::env::var("NOVA_LLM_KEY").unwrap_or_default();
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .ok()?;
        Some(Self {
            endpoint,
            api_key,
            client,
        })
    }
}

#[cfg(feature = "llm")]
impl LlmClient for HttpLlm {
    fn complete(&self, prompt: &str) -> Result<String, LlmError> {
        let body = serde_json::json!({ "prompt": prompt });
        let resp = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .map_err(|e| LlmError::Client(e.to_string()))?;
        let text = resp
            .text()
            .map_err(|e| LlmError::Client(e.to_string()))?;
        Ok(text)
    }
}

/// Extract the first JSON array from `text` (LLMs often wrap the payload in
/// prose or markdown fences) and parse it as an `AgentCommand` batch.
fn parse_commands(text: &str) -> Result<Vec<AgentCommand>, LlmError> {
    let start = text.find('[').ok_or_else(|| {
        LlmError::Client("no JSON array found in LLM output".to_string())
    })?;
    let end = text.rfind(']').ok_or_else(|| {
        LlmError::Client("no JSON array found in LLM output".to_string())
    })?;
    let slice = &text[start..=end];
    Ok(serde_json::from_str(slice)?)
}

/// Apply a fix request by asking `client` to produce an `AgentCommand` batch
/// from the request's prompt plus `context` (RAG-grounded text), then applying
/// the commands to the world through the bounded [`apply_commands`] path.
///
/// Returns the number of commands applied, or an error if the client or parsing
/// failed.
pub fn apply_fix_with_llm(
    world: &mut World,
    req: &AiFixRequest,
    client: &dyn LlmClient,
    context: &str,
) -> Result<usize, LlmError> {
    let prompt = format!("{}\n\nRelevant context:\n{}\n\nRespond with a JSON array of AgentCommands (set_transform/spawn/despawn) only.", req.prompt, context.trim());
    let text = client.complete(&prompt)?;
    let commands = parse_commands(&text)?;
    let n = commands.len();
    apply_commands(world, &commands).map_err(|e| LlmError::Client(e.to_string()))?;
    Ok(n)
}

/// Apply a fix request offline using the built-in heuristic (no network). Used
/// as the default when no real LLM endpoint is configured.
pub fn apply_fix_heuristic(world: &mut World, req: &AiFixRequest) -> Result<usize, LlmError> {
    let commands = fix_request_to_commands(req)?;
    let n = commands.len();
    apply_commands(world, &commands).map_err(|e| LlmError::Client(e.to_string()))?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_ecs::transform::{GlobalTransform, Transform};
    use nova_ecs::Vec3;

    #[test]
    fn heuristic_maps_direction_to_translation() {
        let req = AiFixRequest {
            region: [0.4, 0.4, 0.1, 0.1],
            entity_ids: vec!["e0#0".to_string()],
            instruction: "move the highlighted cube up".to_string(),
            prompt: "FIX REQUEST\nEntities in region: e0#0\nInstruction: move up".to_string(),
        };
        let cmds = fix_request_to_commands(&req).unwrap();
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            AgentCommand::SetTransform {
                target,
                translation,
                ..
            } => {
                assert_eq!(target, &EntityRef::Id("e0#0".to_string()));
                assert_eq!(translation, &Some([0.0, 1.0, 0.0]));
            }
            _ => panic!("expected SetTransform"),
        }
    }

    #[test]
    fn heuristic_client_completes_from_prompt() {
        let client = HeuristicLlm;
        let text = client
            .complete("FIX REQUEST\nEntities in region: e2#0\nInstruction: move the cube left and down")
            .unwrap();
        let cmds: Vec<AgentCommand> = serde_json::from_str(&text).unwrap();
        match &cmds[0] {
            AgentCommand::SetTransform { translation, .. } => {
                assert_eq!(translation, &Some([-1.0, -1.0, 0.0]));
            }
            _ => panic!("expected SetTransform"),
        }
    }

    #[test]
    fn empty_instruction_yields_no_commands() {
        let req = AiFixRequest {
            region: [0.0, 0.0, 0.1, 0.1],
            entity_ids: vec!["e0#0".to_string()],
            instruction: "make it prettier".to_string(),
            prompt: "FIX REQUEST\nEntities in region: e0#0".to_string(),
        };
        assert_eq!(fix_request_to_commands(&req).unwrap().len(), 0);
    }

    #[test]
    fn no_entity_yields_error() {
        let req = AiFixRequest {
            region: [0.0, 0.0, 0.1, 0.1],
            entity_ids: vec![],
            instruction: "up".to_string(),
            prompt: String::new(),
        };
        assert!(fix_request_to_commands(&req).is_err());
    }

    #[test]
    fn apply_fix_heuristic_moves_entity() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::ZERO));
        world.add_component(e, GlobalTransform::identity());

        let req = AiFixRequest {
            region: [0.4, 0.4, 0.1, 0.1],
            entity_ids: vec![format!("{e}")],
            instruction: "move up".to_string(),
            prompt: format!("Entities in region: {e}"),
        };
        let n = apply_fix_heuristic(&mut world, &req).unwrap();
        assert_eq!(n, 1);
        let t = world.get_component::<Transform>(e).unwrap();
        assert!((t.translation.y - 1.0).abs() < 1e-4);
    }

    #[test]
    fn apply_fix_with_llm_parses_markdown_wrapped_array() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::ZERO));
        world.add_component(e, GlobalTransform::identity());

        struct EchoClient;
        impl LlmClient for EchoClient {
            fn complete(&self, _prompt: &str) -> Result<String, LlmError> {
                Ok(format!(
                    "Sure, here are the commands:\n```json\n[{{\"op\":\"set_transform\",\"target\":{{\"id\":\"{e}\"}},\"translation\":[0.0,2.0,0.0],\"rotation_euler_xyz\":null,\"scale\":null}}]\n```"
                ))
            }
        }
        let req = AiFixRequest {
            region: [0.0, 0.0, 0.1, 0.1],
            entity_ids: vec![format!("{e}")],
            instruction: String::new(),
            prompt: String::new(),
        };
        let n = apply_fix_with_llm(&mut world, &req, &EchoClient, "").unwrap();
        assert_eq!(n, 1);
        let t = world.get_component::<Transform>(e).unwrap();
        assert!((t.translation.y - 2.0).abs() < 1e-4);
    }
}
