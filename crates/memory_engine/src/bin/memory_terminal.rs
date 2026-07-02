use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use memory_engine::event::IngestEvent;
use memory_engine::recall::{RecallFilters, RecallQuery};
use memory_engine::types::{EVENT_SCHEMA_VERSION, RECALL_QUERY_SCHEMA_VERSION};
use memory_engine::{FileStorage, MemoryEngine, Result};
use serde_json::json;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

fn main() -> Result<()> {
    let memory_dir = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("memory"));
    let storage = FileStorage::with_host_id(&memory_dir, "memory_terminal");
    storage.ensure_layout()?;

    let mut engine = MemoryEngine::new(storage);
    let mut session_id = "live_terminal".to_string();

    print_intro(&memory_dir, &session_id);

    loop {
        print!("memory> ");
        io::stdout().flush()?;

        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 {
            break;
        }

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        if matches!(input, "/exit" | "/quit") {
            break;
        }

        if input == "/help" {
            print_help(&memory_dir, &session_id);
            continue;
        }

        if input == "/where" {
            println!("memory_dir: {}", memory_dir.display());
            println!("session_id: {session_id}");
            continue;
        }

        if let Some(next_session_id) = input.strip_prefix("/session ") {
            let next_session_id = next_session_id.trim();
            if next_session_id.is_empty() {
                println!("Session id must not be empty.");
            } else {
                session_id = next_session_id.to_string();
                println!("Active session: {session_id}");
            }
            continue;
        }

        if input == "/sleep" {
            match engine.sleep(&session_id) {
                Ok(result) => {
                    println!("Archive created: {}", result.archive_entry.archive_id);
                    println!("Pending task: {}", result.pending_task.task_id);
                    println!("Prompt: prompts/{}.md", result.pending_task.prompt_id);
                }
                Err(error) => println!("Sleep failed: {error}"),
            }
            continue;
        }

        if input == "/tasks" {
            match engine.pending_tasks() {
                Ok(tasks) if tasks.is_empty() => println!("No pending tasks."),
                Ok(tasks) => {
                    for task in tasks {
                        println!(
                            "{} {:?} {:?} prompt={} schema={}",
                            task.task_id,
                            task.task_type,
                            task.state,
                            task.prompt_id,
                            task.expected_output_schema
                        );
                    }
                }
                Err(error) => println!("Could not load tasks: {error}"),
            }
            continue;
        }

        if let Some(query) = input.strip_prefix("/recall ") {
            let query = query.trim();
            if query.is_empty() {
                println!("Recall query must not be empty.");
            } else {
                run_recall(&mut engine, &session_id, query, true)?;
            }
            continue;
        }

        let ingest_result = engine.ingest(IngestEvent {
            schema_version: EVENT_SCHEMA_VERSION.to_string(),
            event_type: "user_message".to_string(),
            source: "terminal_user".to_string(),
            timestamp: now_rfc3339()?,
            session_id: session_id.clone(),
            payload: json!({ "text": input }),
            tags: vec!["terminal_message".to_string()],
            theme: Some("terminal_conversation".to_string()),
            emotional_tone: None,
            speaker: None,
            links: Vec::new(),
            importance_hint: Default::default(),
            processing_mode: Default::default(),
        })?;
        let stored = ingest_result.stored_event;

        println!(
            "Stored event: {} weight={:.2}",
            stored.event_id, stored.initial_weight
        );
        run_recall(&mut engine, &session_id, input, false)?;
    }

    println!("Bye.");
    Ok(())
}

fn print_intro(memory_dir: &Path, session_id: &str) {
    println!("Memory Engine terminal");
    println!("memory_dir: {}", memory_dir.display());
    println!("session_id: {session_id}");
    println!("Type plain text to store it as a memory event.");
    println!("Use /sleep to create archive memory, /recall <text> to search, /help for commands.");
}

fn print_help(memory_dir: &Path, session_id: &str) {
    println!("Commands:");
    println!("  /help              Show this help.");
    println!("  /where             Show active memory directory and session.");
    println!("  /session <id>      Switch active session.");
    println!("  /sleep             Compress current session into preliminary archive memory.");
    println!("  /recall <text>     Search archive memory.");
    println!("  /tasks             Show pending LLM tasks.");
    println!("  /exit              Exit terminal.");
    println!();
    println!("Current memory_dir: {}", memory_dir.display());
    println!("Current session_id: {session_id}");
}

fn run_recall(
    engine: &mut MemoryEngine<FileStorage>,
    session_id: &str,
    query_text: &str,
    explain: bool,
) -> Result<()> {
    let result = engine.recall(RecallQuery {
        schema_version: RECALL_QUERY_SCHEMA_VERSION.to_string(),
        query_id: None,
        created_at: Some(now_rfc3339()?),
        session_id: Some(session_id.to_string()),
        context: json!({ "recent_text": query_text }),
        query_text: Some(query_text.to_string()),
        filters: RecallFilters::default(),
        limit: 5,
        include_core: false,
        explain,
    })?;

    if result.items.is_empty() {
        println!("Recall: no archive memory found yet. Use /sleep after adding facts.");
        return Ok(());
    }

    println!("Recall:");
    for (index, item) in result.items.iter().enumerate() {
        println!(
            "  {}. [{} {:.2}] {}",
            index + 1,
            item.id,
            item.relevance_score,
            item.gist
        );
        if explain {
            if let Some(explanation) = &item.relevance_explanation {
                println!("     {explanation}");
            }
        }
    }

    Ok(())
}

fn now_rfc3339() -> Result<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|err| memory_engine::MemoryEngineError::Storage(err.to_string()))
}
