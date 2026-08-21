use agents::{generate_system_prompt, TechStack};
use anyhow::{bail, Context, Result};
use llm_sdk::llama_cpp::client::LlamaCppClient;
use llm_sdk::llama_cpp::types::LlamaCppChatCompletionResponse;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use crate::RecordArgs;

const DEFAULT_BASE_URL: &str = "http://localhost:8080";

#[derive(Serialize, Deserialize)]
struct PromptRecord {
    name: String,
    fixture: String,
    prompt_version: String,
    prompt: String,
    system_prompt: String,
    model: String,
    base_url: String,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    recorded_at: String,
}

#[derive(Serialize, Deserialize)]
struct RecordedResponse {
    latency_ms: u128,
    response: LlamaCppChatCompletionResponse,
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn recordings_dir() -> PathBuf {
    manifest_dir().join("recordings")
}

fn fixtures_dir() -> PathBuf {
    manifest_dir()
        .parent()
        .expect("evals crate has a parent directory")
        .join("fixtures")
}

pub async fn run(args: RecordArgs) -> Result<()> {
    let prompt = match (&args.prompt, &args.prompt_file) {
        (Some(text), None) => text.clone(),
        (None, Some(path)) => fs::read_to_string(path)
            .with_context(|| format!("reading prompt file {}", path.display()))?,
        (None, None) => bail!("pass either --prompt \"<text>\" or --prompt-file <path>"),
        (Some(_), Some(_)) => unreachable!("clap enforces these are mutually exclusive"),
    };

    let fixture_path = fixtures_dir().join(&args.fixture);
    if !fixture_path.is_dir() {
        bail!(
            "no fixture named '{}' at {} — create it first (see DEVELOP.md)",
            args.fixture,
            fixture_path.display()
        );
    }

    let recording_dir = recordings_dir().join(&args.name);
    if recording_dir.exists() && !args.force {
        bail!(
            "recording '{}' already exists at {} — pass --force to re-record",
            args.name,
            recording_dir.display()
        );
    }

    let base_url = args
        .base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());

    // No real "recent changes" history source exists yet (store/git are both deferred —
    // see epics/002-system-prompt-generator.md), so record calls the generator with an
    // empty history for now.
    let system_prompt = generate_system_prompt(&TechStack::V1, &[]);

    let mut client = LlamaCppClient::new().context("building llama.cpp client")?;
    client = client.with_base_url(base_url.clone());

    let mut builder = client
        .message_builder()
        .model(args.model.clone())
        .system_message(system_prompt.clone())
        .user_message(prompt.clone());
    if let Some(temperature) = args.temperature {
        builder = builder.temperature(temperature);
    }
    if let Some(max_tokens) = args.max_tokens {
        builder = builder.max_tokens(max_tokens);
    }

    println!(
        "sending prompt to {} (model \"{}\")...",
        base_url, args.model
    );
    let started = Instant::now();
    let response = builder
        .send()
        .await
        .with_context(|| format!("calling llama.cpp server at {}", base_url))?;
    let latency_ms = started.elapsed().as_millis();

    fs::create_dir_all(&recording_dir)
        .with_context(|| format!("creating {}", recording_dir.display()))?;

    let prompt_record = PromptRecord {
        name: args.name.clone(),
        fixture: args.fixture.clone(),
        prompt_version: args.prompt_version.clone(),
        prompt,
        system_prompt,
        model: args.model.clone(),
        base_url,
        temperature: args.temperature,
        max_tokens: args.max_tokens,
        recorded_at: chrono::Utc::now().to_rfc3339(),
    };
    fs::write(
        recording_dir.join("prompt.toml"),
        toml::to_string_pretty(&prompt_record).context("serializing prompt.toml")?,
    )
    .context("writing prompt.toml")?;

    let recorded_response = RecordedResponse {
        latency_ms,
        response: response.clone(),
    };
    fs::write(
        recording_dir.join("response.json"),
        serde_json::to_string_pretty(&recorded_response).context("serializing response.json")?,
    )
    .context("writing response.json")?;

    let reply_text = response
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone())
        .unwrap_or_default();

    println!(
        "recorded to {} ({} ms)\n---\n{}\n---",
        recording_dir.display(),
        latency_ms,
        reply_text
    );

    Ok(())
}
