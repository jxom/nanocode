#[cfg(not(target_family = "wasm"))]
#[path = "support/agents.rs"]
mod agents;
#[cfg(not(target_family = "wasm"))]
mod support {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
}

/// A short assessment of a Rust snippet.
#[cfg(not(target_family = "wasm"))]
#[nanocode::output]
#[derive(Debug)]
struct Review {
    /// Whether the snippet can merge as written.
    verdict: Verdict,
    /// One paragraph of plain prose.
    #[schemars(length(max = 600))]
    summary: String,
    /// Each problem found, most severe first.
    issues: Vec<Issue>,
}

#[cfg(not(target_family = "wasm"))]
#[nanocode::output]
#[derive(Debug)]
#[serde(rename_all = "snake_case")]
enum Verdict {
    Approve,
    RequestChanges,
}

#[cfg(not(target_family = "wasm"))]
#[nanocode::output]
#[derive(Debug)]
struct Issue {
    /// 1-based line number in the snippet.
    #[schemars(range(min = 1))]
    line: u32,
    severity: Severity,
    message: String,
}

#[cfg(not(target_family = "wasm"))]
#[nanocode::output]
#[derive(Debug)]
#[serde(rename_all = "snake_case")]
enum Severity {
    Low,
    Medium,
    High,
}

#[cfg(not(target_family = "wasm"))]
const SNIPPET: &str =
    "fn average(values: &[i64]) -> i64 {\n    values.iter().sum::<i64>() / values.len() as i64\n}";

#[cfg(not(target_family = "wasm"))]
async fn run() -> support::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--help") {
        println!(
            "cargo run --locked -p nanocode --example output -- [claude|codex|--schema]\nRequests a typed Review from one agent turn through submit_output.\nSet NANOCODE_EXAMPLE_API_BASE plus both API keys to use a compatible local endpoint."
        );
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "--schema") {
        let schema = nanocode::output::schema::<Review>();
        println!("{}", serde_json::to_string_pretty(&schema)?);
        return Ok(());
    }
    let backend = args.first().map_or("claude", String::as_str);
    let outputs = nanocode::Outputs::new();
    let tools = nanocode::Tools::builder()
        .without_defaults()
        .tool(outputs.tool())
        .build()?;
    let runtime = agents::runtime(tools)?.build_async().await?;
    let outcome: support::Result<()> = async {
        let (agent, _events) = agents::spawn(&runtime, backend).await?;
        let turn = outputs
            .prompt::<Review>(
                &agent,
                format!("Review this Rust function:\n\n```rust\n{SNIPPET}\n```"),
            )
            .await?;
        let review = turn.output;
        println!("{:?}: {}", review.verdict, review.summary);
        for issue in review.issues {
            println!(
                "  line {} ({:?}): {}",
                issue.line, issue.severity, issue.message
            );
        }
        agent.flush().await?;
        Ok(())
    }
    .await;
    let shutdown = runtime.shutdown().await;
    outcome?;
    shutdown?;
    Ok(())
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    let outcome = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(Into::into)
        .and_then(|runtime| runtime.block_on(run()));
    if let Err(error) = outcome {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(target_family = "wasm")]
fn main() {}
