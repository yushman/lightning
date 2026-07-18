use std::path::Path;

use crate::affected::{EXIT_STALE, Outcome, SelectArgs, select};
use crate::sync;

#[derive(clap::Args)]
pub struct RunArgs {
    /// Task name, matched exactly against each affected module's task list
    pub task: String,
    #[command(flatten)]
    pub select: SelectArgs,
    /// Extra arguments passed to Gradle verbatim
    #[arg(last = true)]
    pub gradle_args: Vec<String>,
}

pub fn run(dir: &Path, args: &RunArgs) -> Result<i32, String> {
    let selection = match select(dir, &args.select)? {
        Outcome::Selected(s) => s,
        Outcome::Stale(message) => {
            eprintln!("error: {message}");
            return Ok(EXIT_STALE);
        }
    };
    if let Some(reason) = &selection.everything {
        eprintln!("lightning: warning: {reason}; running on everything");
    }
    if selection.modules.is_empty() {
        println!("lightning: nothing affected, skipping gradle");
        return Ok(0);
    }
    let mut task_paths: Vec<String> = Vec::new();
    for (path, _) in &selection.modules {
        let module = selection
            .lock
            .modules
            .iter()
            .find(|m| &m.path == path)
            .expect("selected module is in the lock");
        if module.tasks.iter().any(|t| t == &args.task) {
            task_paths.push(if path == ":" {
                format!(":{}", args.task)
            } else {
                format!("{path}:{}", args.task)
            });
        } else {
            eprintln!("lightning: {path} has no task {:?}, skipped", args.task);
        }
    }
    if task_paths.is_empty() {
        println!(
            "lightning: no affected module has task {:?}, skipping gradle",
            args.task
        );
        return Ok(0);
    }
    eprintln!("lightning: running {}", task_paths.join(" "));
    let status = sync::gradle_command(dir)
        .current_dir(dir)
        .args(&task_paths)
        .args(&args.gradle_args)
        .status()
        .map_err(|e| format!("cannot run gradle: {e}"))?;
    Ok(status.code().unwrap_or(1))
}
