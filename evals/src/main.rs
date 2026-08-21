mod record;

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "evals", about = "Record and replay LLM interactions for protopie")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Send a prompt to a live local LLM and store the prompt + raw response.
    Record(RecordArgs),
    /// Run assertions against a stored recording (not implemented yet).
    Test(TestArgs),
}

#[derive(Args)]
struct RecordArgs {
    /// Name for this recording (used as the directory name under evals/recordings/).
    name: String,

    /// Name of the sample app under fixtures/ this prompt is aimed at.
    #[arg(long)]
    fixture: String,

    /// Prompt text, given directly on the command line.
    #[arg(long, conflicts_with = "prompt_file")]
    prompt: Option<String>,

    /// Prompt text, read from a file (for longer brain-dumps).
    #[arg(long = "prompt-file", conflicts_with = "prompt")]
    prompt_file: Option<PathBuf>,

    /// Free-form label for the prompt's phrasing/version, stored alongside the recording.
    #[arg(long, default_value = "v0")]
    prompt_version: String,

    /// Model name sent to the llama.cpp server (most local servers ignore this).
    #[arg(long, default_value = "local")]
    model: String,

    /// Override the llama.cpp server base URL (defaults to http://localhost:8080).
    #[arg(long)]
    base_url: Option<String>,

    #[arg(long)]
    temperature: Option<f32>,

    #[arg(long)]
    max_tokens: Option<u32>,

    /// Overwrite an existing recording with the same name.
    #[arg(long)]
    force: bool,
}

#[derive(Args)]
struct TestArgs {
    /// Name of a single recording to test; omit to run all recordings that have assertions.
    name: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Record(args) => record::run(args).await,
        Command::Test(_) => {
            anyhow::bail!(
                "`evals test` is not implemented yet — this is the next task in epics/001-evolution-replay-harness.md"
            )
        }
    }
}
