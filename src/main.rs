use clap::{Parser, Subcommand};
use colored::Colorize;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod colorize;
mod config;
mod dwl_highlight;
mod flatten;
mod history;
mod picker;
mod repl;
mod run_once;
mod sidecar;
mod status;
mod watch;

/// blazewvr — fast local playground for DataWeave scripts
#[derive(Parser)]
#[command(name = "blazewvr", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a DataWeave script against one or more inputs
    Run {
        script: String,
        #[arg(short, long)]
        input: Vec<String>,
        /// Module resolution path(s), e.g. --path=dir1:dir2
        #[arg(long)]
        path: Option<String>,
    },
    /// Interactive read-eval-print loop
    Repl {
        /// Module resolution path(s), e.g. --path=dir1:dir2
        #[arg(long)]
        path: Option<String>,
    },
    /// Validate a DataWeave script without running it
    Validate { script: String },
    /// Watch a script (and its inputs) and re-evaluate on every save
    Watch {
        /// Script to watch; omit to fuzzy-pick one via fzf
        script: Option<String>,
        #[arg(short, long)]
        input: Vec<String>,
        /// Module resolution path(s), e.g. --path=dir1:dir2
        #[arg(long)]
        path: Option<String>,
        /// Fuzzy-pick input files via fzf instead of -i/sidecar/config
        #[arg(long)]
        pick_inputs: bool,
    },
    /// Fuzzy-recall a past `watch` invocation and re-run it
    History,
}

/// Renders the (currently stubbed) response for a parsed command.
fn dispatch(command: Commands) -> String {
    match command {
        Commands::Run {
            script,
            input,
            path,
        } => {
            format!(
                "{} {script} (inputs: {input:?}, path: {path:?})",
                "[run] not yet implemented:".yellow()
            )
        }
        Commands::Repl { path } => {
            format!("{} (path: {path:?})", "[repl] not yet implemented".yellow())
        }
        Commands::Validate { script } => {
            format!("{} {script}", "[validate] not yet implemented:".yellow())
        }
        Commands::Watch { .. } | Commands::History => {
            unreachable!("Watch/History are handled directly in main(), not dispatch()")
        }
    }
}

/// CLI `--path` wins over the config file's `module_path`.
fn resolve_module_path(cli_path: Option<String>, config_path: Option<String>) -> Option<String> {
    cli_path.or(config_path)
}

/// Explicit `-i` flags win outright; otherwise sidecar auto-discovery;
/// otherwise the config file's default inputs.
fn resolve_inputs(
    cli_inputs: Vec<(String, PathBuf)>,
    sidecar_inputs: Vec<(String, PathBuf)>,
    config_inputs: Vec<(String, PathBuf)>,
) -> Vec<(String, PathBuf)> {
    if !cli_inputs.is_empty() {
        cli_inputs
    } else if !sidecar_inputs.is_empty() {
        sidecar_inputs
    } else {
        config_inputs
    }
}

/// Prompts on stdin for a name to bind a picked input file as, falling
/// back to the picker's own default (the file's stem) on a blank answer.
fn prompt_input_name(default_name: &str) -> std::io::Result<String> {
    use std::io::Write;
    print!("name for {default_name} [{default_name}]: ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

/// Best-effort history recording at an explicit path: a failure to write
/// history (e.g. unwritable dir) should never fail the `watch` command
/// itself.
fn record_history_at(
    path: &Path,
    script: &Path,
    inputs: &[(String, PathBuf)],
    module_path: &Option<String>,
) {
    let entry =
        history::HistoryEntry::now(script.to_path_buf(), inputs.to_vec(), module_path.clone());
    let _ = history::append(path, entry, 100);
}

/// The fully-resolved script/inputs/module-path a `watch` invocation will
/// run against, after applying script/input picking (if requested) and
/// the CLI > sidecar > config precedence chain.
struct WatchPlan {
    script_path: PathBuf,
    inputs: Vec<(String, PathBuf)>,
    module_path: Option<String>,
}

/// Resolves everything `run_watch` needs, in isolation from process
/// spawning (`picker_config`/`name_for` are injected so this is testable
/// against a fake picker script instead of real `fzf`).
fn prepare_watch(
    cwd: &Path,
    script: Option<String>,
    input: Vec<String>,
    path: Option<String>,
    pick_inputs: bool,
    picker_config: &picker::PickerConfig,
    name_for: impl FnMut(&str) -> std::io::Result<String>,
) -> anyhow::Result<WatchPlan> {
    let project = config::load(cwd)?;

    let script_path = match script {
        Some(s) => PathBuf::from(s),
        None => picker::pick_script(picker_config, cwd)?
            .ok_or_else(|| anyhow::anyhow!("no script selected"))?,
    };

    let cli_inputs: Vec<(String, PathBuf)> = input
        .iter()
        .map(|s| watch::parse_input(s))
        .collect::<Result<_, _>>()
        .map_err(anyhow::Error::msg)?;

    let inputs = if pick_inputs {
        picker::bind_inputs_via_picker(picker_config, &script_path, cwd, name_for)?
    } else {
        let sidecar_inputs = sidecar::discover(&script_path);
        let config_inputs = project
            .as_ref()
            .map(|p| p.inputs.clone())
            .unwrap_or_default();
        resolve_inputs(cli_inputs, sidecar_inputs, config_inputs)
    };

    let module_path = resolve_module_path(path, project.and_then(|p| p.module_path));

    Ok(WatchPlan {
        script_path,
        inputs,
        module_path,
    })
}

fn build_repl_config(
    inputs: &[(String, PathBuf)],
    module_path: &Option<String>,
) -> repl::ReplConfig {
    let mut config = repl::ReplConfig::for_dw(Path::new("dw"), inputs);
    if let Some(p) = module_path {
        config = config.with_module_path(p);
    }
    config
}

/// Builds the supervisor + watch target and runs the poll loop forever
/// (until the process is killed), printing each eval's result.
fn run_watch(
    script: Option<String>,
    input: Vec<String>,
    path: Option<String>,
    pick_inputs: bool,
) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let plan = prepare_watch(
        &cwd,
        script,
        input,
        path,
        pick_inputs,
        &picker::PickerConfig::default(),
        prompt_input_name,
    )?;

    let repl_config = build_repl_config(&plan.inputs, &plan.module_path);
    if let Some(history_path) = history::default_history_path() {
        record_history_at(
            &history_path,
            &plan.script_path,
            &plan.inputs,
            &plan.module_path,
        );
    }

    let target = watch::WatchTarget {
        script: plan.script_path,
        inputs: plan.inputs,
    };
    watch::run_loop(
        repl::Supervisor::new(repl_config),
        &target,
        Duration::from_millis(300),
        |out| println!("{out}\n"),
        || true,
    );
    Ok(())
}

/// Fuzzy-recalls a past `watch` invocation via `fzf` and re-runs it.
fn run_history() -> anyhow::Result<()> {
    let Some(path) = history::default_history_path() else {
        anyhow::bail!("no $HOME; can't locate history file");
    };
    let picker_config = picker::PickerConfig::default();
    let Some(entry) = history::pick_history(&picker_config, &path)? else {
        return Ok(());
    };

    let repl_config = build_repl_config(&entry.inputs, &entry.module_path);
    let target = watch::WatchTarget {
        script: entry.script,
        inputs: entry.inputs,
    };
    watch::run_loop(
        repl::Supervisor::new(repl_config),
        &target,
        Duration::from_millis(300),
        |out| println!("{out}\n"),
        || true,
    );
    Ok(())
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Commands::Watch {
            script,
            input,
            path,
            pick_inputs,
        } => run_watch(script, input, path, pick_inputs),
        Commands::History => run_history(),
        other => {
            println!("{}", dispatch(other));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn test_dir() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("blazewvr_main_{}_{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    /// Fake fzf: prints the first stdin line, ignoring all args (`-m`
    /// etc.) so both single- and multi-select calls resolve deterministically.
    fn fake_fzf_script() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "blazewvr_main_fake_fzf_{}_{id}.sh",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nhead -n1\n").expect("write fake fzf script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    fn no_prompt(default_name: &str) -> std::io::Result<String> {
        Ok(default_name.to_string())
    }

    #[test]
    fn prepare_watch_uses_explicit_script_without_picker() {
        let dir = test_dir();
        let picker_config = picker::PickerConfig {
            program: PathBuf::from("/nonexistent/blazewvr/nope"),
            ..Default::default()
        };
        let plan = prepare_watch(
            &dir,
            Some("s.dwl".into()),
            vec![],
            None,
            false,
            &picker_config,
            no_prompt,
        )
        .unwrap();
        assert_eq!(plan.script_path, PathBuf::from("s.dwl"));
        assert!(plan.inputs.is_empty());
        assert_eq!(plan.module_path, None);
    }

    #[test]
    fn prepare_watch_picks_script_when_none_given() {
        let dir = test_dir();
        fs::write(dir.join("a.dwl"), "x").unwrap();
        let picker_config = picker::PickerConfig {
            program: fake_fzf_script(),
            ..Default::default()
        };
        let plan =
            prepare_watch(&dir, None, vec![], None, false, &picker_config, no_prompt).unwrap();
        assert_eq!(plan.script_path, dir.join("a.dwl"));
    }

    #[test]
    fn prepare_watch_errs_when_no_script_selected() {
        let dir = test_dir();
        let picker_config = picker::PickerConfig {
            program: fake_fzf_script(),
            ..Default::default()
        };
        let result = prepare_watch(&dir, None, vec![], None, false, &picker_config, no_prompt);
        assert!(result.is_err());
    }

    #[test]
    fn prepare_watch_uses_sidecar_inputs_when_no_cli_inputs() {
        let dir = test_dir();
        let script = dir.join("s.dwl");
        fs::write(&script, "x").unwrap();
        fs::write(dir.join("s.payload.json"), "{}").unwrap();
        let picker_config = picker::PickerConfig {
            program: PathBuf::from("/nonexistent/blazewvr/nope"),
            ..Default::default()
        };
        let plan = prepare_watch(
            &dir,
            Some(script.display().to_string()),
            vec![],
            None,
            false,
            &picker_config,
            no_prompt,
        )
        .unwrap();
        assert_eq!(
            plan.inputs,
            vec![("payload".to_string(), dir.join("s.payload.json"))]
        );
    }

    #[test]
    fn prepare_watch_uses_config_module_path_when_no_cli_path() {
        let dir = test_dir();
        fs::write(dir.join("blazewvr.toml"), "module_path = \"libs\"\n").unwrap();
        let picker_config = picker::PickerConfig {
            program: PathBuf::from("/nonexistent/blazewvr/nope"),
            ..Default::default()
        };
        let plan = prepare_watch(
            &dir,
            Some("s.dwl".into()),
            vec![],
            None,
            false,
            &picker_config,
            no_prompt,
        )
        .unwrap();
        assert_eq!(plan.module_path, Some("libs".to_string()));
    }

    #[test]
    fn prepare_watch_cli_path_wins_over_config() {
        let dir = test_dir();
        fs::write(dir.join("blazewvr.toml"), "module_path = \"libs\"\n").unwrap();
        let picker_config = picker::PickerConfig {
            program: PathBuf::from("/nonexistent/blazewvr/nope"),
            ..Default::default()
        };
        let plan = prepare_watch(
            &dir,
            Some("s.dwl".into()),
            vec![],
            Some("cli-dir".into()),
            false,
            &picker_config,
            no_prompt,
        )
        .unwrap();
        assert_eq!(plan.module_path, Some("cli-dir".to_string()));
    }

    #[test]
    fn prepare_watch_rejects_invalid_input_format() {
        let dir = test_dir();
        let picker_config = picker::PickerConfig {
            program: PathBuf::from("/nonexistent/blazewvr/nope"),
            ..Default::default()
        };
        let result = prepare_watch(
            &dir,
            Some("s.dwl".into()),
            vec!["bad-input".into()],
            None,
            false,
            &picker_config,
            no_prompt,
        );
        assert!(result.is_err());
    }

    #[test]
    fn prepare_watch_binds_inputs_via_picker_when_pick_inputs_set() {
        let dir = test_dir();
        let script = dir.join("s.dwl");
        fs::write(&script, "x").unwrap();
        fs::write(dir.join("payload.json"), "{}").unwrap();
        let picker_config = picker::PickerConfig {
            program: fake_fzf_script(),
            ..Default::default()
        };
        let plan = prepare_watch(
            &dir,
            Some(script.display().to_string()),
            vec![],
            None,
            true,
            &picker_config,
            no_prompt,
        )
        .unwrap();
        assert_eq!(
            plan.inputs,
            vec![("payload".to_string(), dir.join("payload.json"))]
        );
    }

    #[test]
    fn build_repl_config_applies_module_path() {
        let config = build_repl_config(&[], &Some("libs".to_string()));
        assert!(config.args.iter().any(|a| a.contains("libs")));
    }

    #[test]
    fn build_repl_config_omits_path_flag_when_none() {
        let config = build_repl_config(&[], &None);
        assert!(!config.args.iter().any(|a| a == "--path"));
    }

    #[test]
    fn record_history_at_writes_an_entry() {
        let dir = test_dir();
        let history_path = dir.join("history.jsonl");
        record_history_at(&history_path, Path::new("s.dwl"), &[], &None);
        let entries = history::load(&history_path);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].script, PathBuf::from("s.dwl"));
    }

    #[test]
    fn run_reports_script_and_inputs() {
        let out = dispatch(Commands::Run {
            script: "output json --- payload".into(),
            input: vec!["payload.json".into()],
            path: None,
        });
        assert!(out.contains("output json --- payload"));
        assert!(out.contains("payload.json"));
    }

    #[test]
    fn run_reports_module_path() {
        let out = dispatch(Commands::Run {
            script: "payload".into(),
            input: vec![],
            path: Some("dir1:dir2".into()),
        });
        assert!(out.contains("dir1:dir2"));
    }

    #[test]
    fn repl_reports_stub() {
        let out = dispatch(Commands::Repl { path: None });
        assert!(out.contains("not yet implemented"));
    }

    #[test]
    fn repl_reports_module_path() {
        let out = dispatch(Commands::Repl {
            path: Some("dir1".into()),
        });
        assert!(out.contains("dir1"));
    }

    #[test]
    fn cli_parses_path_flag() {
        let cli = Cli::parse_from(["blazewvr", "run", "s.dwl", "--path", "a:b"]);
        match cli.command {
            Commands::Run { path, .. } => assert_eq!(path, Some("a:b".to_string())),
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn validate_reports_script() {
        let out = dispatch(Commands::Validate {
            script: "foo.dwl".into(),
        });
        assert!(out.contains("foo.dwl"));
    }

    #[test]
    fn run_watch_rejects_invalid_input_format() {
        let result = run_watch(
            Some("script.dwl".into()),
            vec!["bad-input".into()],
            None,
            false,
        );
        assert!(result.is_err());
    }

    #[test]
    fn cli_parses_watch_subcommand() {
        let cli = Cli::parse_from(["blazewvr", "watch", "s.dwl", "-i", "a=b.json"]);
        match cli.command {
            Commands::Watch { script, input, .. } => {
                assert_eq!(script, Some("s.dwl".to_string()));
                assert_eq!(input, vec!["a=b.json".to_string()]);
            }
            _ => panic!("expected Watch"),
        }
    }

    #[test]
    fn cli_parses_watch_without_script() {
        let cli = Cli::parse_from(["blazewvr", "watch"]);
        match cli.command {
            Commands::Watch { script, .. } => assert_eq!(script, None),
            _ => panic!("expected Watch"),
        }
    }

    #[test]
    fn cli_parses_watch_pick_inputs_flag() {
        let cli = Cli::parse_from(["blazewvr", "watch", "s.dwl", "--pick-inputs"]);
        match cli.command {
            Commands::Watch { pick_inputs, .. } => assert!(pick_inputs),
            _ => panic!("expected Watch"),
        }
    }

    #[test]
    fn cli_parses_history_subcommand() {
        let cli = Cli::parse_from(["blazewvr", "history"]);
        assert!(matches!(cli.command, Commands::History));
    }

    #[test]
    fn resolve_module_path_prefers_cli() {
        assert_eq!(
            resolve_module_path(Some("cli".into()), Some("config".into())),
            Some("cli".into())
        );
    }

    #[test]
    fn resolve_module_path_falls_back_to_config() {
        assert_eq!(
            resolve_module_path(None, Some("config".into())),
            Some("config".into())
        );
    }

    #[test]
    fn resolve_inputs_prefers_cli_over_sidecar_and_config() {
        let cli = vec![("a".to_string(), PathBuf::from("a.json"))];
        let sidecar = vec![("b".to_string(), PathBuf::from("b.json"))];
        let config = vec![("c".to_string(), PathBuf::from("c.json"))];
        assert_eq!(resolve_inputs(cli.clone(), sidecar, config), cli);
    }

    #[test]
    fn resolve_inputs_falls_back_to_sidecar_over_config() {
        let sidecar = vec![("b".to_string(), PathBuf::from("b.json"))];
        let config = vec![("c".to_string(), PathBuf::from("c.json"))];
        assert_eq!(resolve_inputs(vec![], sidecar.clone(), config), sidecar);
    }

    #[test]
    fn resolve_inputs_falls_back_to_config_when_others_empty() {
        let config = vec![("c".to_string(), PathBuf::from("c.json"))];
        assert_eq!(resolve_inputs(vec![], vec![], config.clone()), config);
    }

    #[test]
    fn cli_parses_run_subcommand() {
        let cli = Cli::parse_from(["blazewvr", "run", "script.dwl", "-i", "a.json"]);
        match cli.command {
            Commands::Run { script, input, .. } => {
                assert_eq!(script, "script.dwl");
                assert_eq!(input, vec!["a.json".to_string()]);
            }
            _ => panic!("expected Run"),
        }
    }
}
