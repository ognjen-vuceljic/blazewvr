use clap::{Parser, Subcommand};
use colored::Colorize;
use std::io::BufRead;
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
mod span;
mod status;
mod tui;
mod watch;

/// blazewvr — fast local playground for DataWeave scripts
#[derive(Parser)]
#[command(name = "blazewvr", version)]
struct Cli {
    /// Script to open in the TUI playground (bare invocation only; omit
    /// to open with a placeholder until one is picked)
    script: Option<String>,
    #[arg(short, long)]
    input: Vec<String>,
    /// Module resolution path(s), e.g. --path=dir1:dir2
    #[arg(long)]
    path: Option<String>,
    #[command(subcommand)]
    command: Option<Commands>,
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
        /// Bind an input for expressions to reference, e.g. -i data.json
        /// (defaults to `payload`) or -i name=file for another name
        #[arg(short, long)]
        input: Vec<String>,
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
        Commands::Validate { script } => {
            format!("{} {script}", "[validate] not yet implemented:".yellow())
        }
        Commands::Run { .. }
        | Commands::Repl { .. }
        | Commands::Watch { .. }
        | Commands::History => {
            unreachable!("Run/Repl/Watch/History are handled directly in main(), not dispatch()")
        }
    }
}

/// The fully-resolved script source/inputs/module-path a `run` invocation
/// will evaluate against.
struct RunPlan {
    src: String,
    inputs: Vec<(String, PathBuf)>,
    module_path: Option<String>,
}

/// Resolves everything `run_run` needs (script contents, inputs, module
/// path — CLI > sidecar > config, same precedence as `watch`), kept free
/// of process spawning so it's directly unit-testable.
fn prepare_run(
    cwd: &Path,
    script: &str,
    input: Vec<String>,
    path: Option<String>,
) -> anyhow::Result<RunPlan> {
    let project = config::load(cwd)?;

    let script_path = PathBuf::from(script);
    let src = std::fs::read_to_string(&script_path)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", script_path.display()))?;

    let cli_inputs: Vec<(String, PathBuf)> = input
        .iter()
        .map(|s| watch::parse_input(s))
        .collect::<Result<_, _>>()
        .map_err(anyhow::Error::msg)?;
    let sidecar_inputs = sidecar::discover(&script_path);
    let config_inputs = project
        .as_ref()
        .map(|p| p.inputs.clone())
        .unwrap_or_default();
    let inputs = resolve_inputs(cli_inputs, sidecar_inputs, config_inputs);
    let module_path = resolve_module_path(path, project.and_then(|p| p.module_path));

    Ok(RunPlan {
        src,
        inputs,
        module_path,
    })
}

/// The one-shot input -> script -> output flow: evaluates `script` exactly
/// once via `dw run` and prints the result. Exits non-zero (via the
/// returned `Err`) on a read or eval failure, unlike `watch`, since a
/// one-shot run is the shape a script or pipeline would actually depend
/// on the exit code for.
fn run_run(script: String, input: Vec<String>, path: Option<String>) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let plan = prepare_run(&cwd, &script, input, path)?;

    let run_config =
        run_once::RunOnceConfig::for_dw(Path::new("dw"), &plan.inputs, plan.module_path.as_deref());
    match run_once::eval_once(&run_config, &plan.src) {
        Ok(text) => {
            println!("{}", colorize::colorize(&text));
            Ok(())
        }
        Err(err) => {
            eprintln!("{}", dwl_highlight::highlight(&format!("error: {err}")));
            anyhow::bail!("script evaluation failed")
        }
    }
}

/// Evaluates one line typed at the interactive REPL, returning the
/// colorized/highlighted text to print. Pulled out of `run_repl` (which
/// wraps this in a blocking read-a-line-from-stdin loop, not itself
/// practical to unit test) so the eval + formatting logic is directly
/// testable against a fake `dw`-shaped script.
fn repl_eval_line(sup: &mut repl::Supervisor, line: &str) -> String {
    match sup.eval(line) {
        Ok(text) => colorize::colorize(&text),
        Err(err) => dwl_highlight::highlight(&format!("error: {err}")),
    }
}

/// The interactive layer: a real-time, user-driven read-eval-print loop.
/// Each line the user types is sent straight to a persistent `dw repl`
/// child (via `Supervisor`, so a crashed child transparently respawns)
/// and its result is printed immediately — unlike `run` (one script,
/// one output, exits) or `watch` (re-evaluates a file on save), this is
/// driven by the user typing at the prompt, one expression at a time.
fn run_repl(input: Vec<String>, path: Option<String>) -> anyhow::Result<()> {
    let inputs: Vec<(String, PathBuf)> = input
        .iter()
        .map(|s| watch::parse_input(s))
        .collect::<Result<_, _>>()
        .map_err(anyhow::Error::msg)?;

    let mut config = repl::ReplConfig::for_dw(Path::new("dw"), &inputs);
    if let Some(p) = &path {
        config = config.with_module_path(p);
    }
    let mut sup = repl::Supervisor::new(config);

    if inputs.is_empty() {
        println!(
            "blazewvr REPL — no inputs bound (use -i data.json to bind `payload`), type a DataWeave expression, Ctrl+D to exit"
        );
    } else {
        let bound = inputs
            .iter()
            .map(|(name, file)| format!("{name}={}", file.display()))
            .collect::<Vec<_>>()
            .join(", ");
        println!("blazewvr REPL — inputs: {bound} — type a DataWeave expression, Ctrl+D to exit");
    }
    let stdin = std::io::stdin();
    loop {
        print!("dw> ");
        std::io::Write::flush(&mut std::io::stdout())?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            println!();
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        println!("{}", repl_eval_line(&mut sup, line));
    }
    Ok(())
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

/// The placeholder content shown until a script is picked (bare
/// `blazewvr`, no script, no subcommand).
fn static_playground_state() -> tui::PlaygroundState {
    tui::PlaygroundState {
        script_spans: dwl_highlight::tokenize("%dw 2.0\noutput application/json\n---\n{}"),
        inputs: vec![(
            "no input bound (use -i data.json to bind `payload`)".to_string(),
            Vec::new(),
        )],
        active_input: 0,
        output_spans: Vec::new(),
    }
}

/// Renders the static placeholder once and exits on any keypress.
/// Picker integration lands in a later Wave 4 PR (issue #6) — for now
/// this is the only bare-with-no-script behavior.
fn run_playground_static() -> anyhow::Result<()> {
    let mut terminal = ratatui::try_init()?;
    let state = static_playground_state();
    terminal.draw(|frame| tui::render(frame, &state))?;
    crossterm::event::read()?;
    ratatui::restore();
    Ok(())
}

/// One tab per bound input: its name + highlighted spans of its raw
/// file contents. Always returns at least one entry — a placeholder
/// tab if no inputs are bound — so `PlaygroundState.inputs` never needs
/// to special-case "empty" at render time.
fn playground_inputs_display(inputs: &[(String, PathBuf)]) -> Vec<(String, Vec<span::StyledSpan>)> {
    if inputs.is_empty() {
        return vec![(
            "no input bound (use -i data.json to bind `payload`)".to_string(),
            Vec::new(),
        )];
    }
    inputs
        .iter()
        .map(|(name, file)| {
            let text = std::fs::read_to_string(file)
                .unwrap_or_else(|e| format!("error reading {}: {e}", file.display()));
            (name.clone(), colorize::tokenize(&text))
        })
        .collect()
}

/// Re-reads the script, re-evaluates it against `target`'s bound inputs,
/// and builds the highlighted `PlaygroundState` to render for this tick
/// (`active_input` is left at 0; `run_playground_ticks` owns which tab
/// is selected across ticks and overwrites it before drawing). Pulled
/// out of the render loop (which owns the real terminal/crossterm
/// input, not itself practical to unit test) so this eval + highlighting
/// logic is directly testable against a fake `dw`-shaped script.
fn build_playground_state(
    sup: &mut repl::Supervisor,
    target: &watch::WatchTarget,
) -> tui::PlaygroundState {
    let script_src = std::fs::read_to_string(&target.script).unwrap_or_default();
    let inputs = playground_inputs_display(&target.inputs);
    let output_spans = match sup.eval(&flatten::flatten(&script_src)) {
        Ok(text) => colorize::tokenize(&text),
        Err(err) => dwl_highlight::tokenize(&format!("error: {err}")),
    };
    tui::PlaygroundState {
        script_spans: dwl_highlight::tokenize(&script_src),
        inputs,
        active_input: 0,
        output_spans,
    }
}

/// The live-reload loop's branching: quit on `q`/Esc, cycle the active
/// input tab on `Tab`/`Right`/`l` (forward) or `BackTab`/`Left`/`h`
/// (back), re-evaluate on a watched-file change, otherwise keep looping
/// — same 300ms poll cadence as `watch`. `draw`/`poll_key` are injected
/// (rather than owning a real terminal/crossterm input directly) so
/// this is testable with fakes; `run_playground_live_loop` is the thin
/// real-IO adapter around it.
fn run_playground_ticks(
    sup: &mut repl::Supervisor,
    target: &watch::WatchTarget,
    mut draw: impl FnMut(&tui::PlaygroundState) -> anyhow::Result<()>,
    mut poll_key: impl FnMut(Duration) -> anyhow::Result<Option<crossterm::event::KeyCode>>,
) -> anyhow::Result<()> {
    use crossterm::event::KeyCode;

    let paths = watch::watched_paths(target);
    let mut last = watch::mtimes(&paths);
    let mut state = build_playground_state(sup, target);
    let mut active_input = 0usize;

    loop {
        state.active_input = active_input;
        draw(&state)?;

        match poll_key(Duration::from_millis(300))? {
            Some(KeyCode::Char('q')) | Some(KeyCode::Esc) => return Ok(()),
            Some(KeyCode::Tab) | Some(KeyCode::Right) | Some(KeyCode::Char('l')) => {
                active_input = (active_input + 1) % state.inputs.len();
            }
            Some(KeyCode::BackTab) | Some(KeyCode::Left) | Some(KeyCode::Char('h')) => {
                active_input = active_input
                    .checked_sub(1)
                    .unwrap_or(state.inputs.len() - 1);
            }
            Some(_) => {}
            None => {
                let current = watch::mtimes(&paths);
                if current != last {
                    last = current;
                    state = build_playground_state(sup, target);
                    active_input = active_input.min(state.inputs.len() - 1);
                }
            }
        }
    }
}

/// Not unit-testable (owns the real terminal + crossterm input); see
/// `run_playground_ticks` for the tested branching logic this wraps.
fn run_playground_live_loop(
    terminal: &mut ratatui::DefaultTerminal,
    sup: &mut repl::Supervisor,
    target: &watch::WatchTarget,
) -> anyhow::Result<()> {
    run_playground_ticks(
        sup,
        target,
        |state| {
            terminal.draw(|frame| tui::render(frame, state))?;
            Ok(())
        },
        |timeout| {
            if crossterm::event::poll(timeout)?
                && let crossterm::event::Event::Key(key) = crossterm::event::read()?
            {
                return Ok(Some(key.code));
            }
            Ok(None)
        },
    )
}

/// Bare `blazewvr script.dwl`: opens the TUI playground live-reloading
/// that script, same input resolution (CLI > sidecar > config) as `run`.
fn run_playground_live(
    script: String,
    input: Vec<String>,
    path: Option<String>,
) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let plan = prepare_run(&cwd, &script, input, path)?;
    let target = watch::WatchTarget {
        script: PathBuf::from(&script),
        inputs: plan.inputs.clone(),
    };
    let mut sup = repl::Supervisor::new(build_repl_config(&plan.inputs, &plan.module_path));

    let mut terminal = ratatui::try_init()?;
    let result = run_playground_live_loop(&mut terminal, &mut sup, &target);
    ratatui::restore();
    result
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None => match cli.script {
            Some(script) => run_playground_live(script, cli.input, cli.path),
            None => run_playground_static(),
        },
        Some(Commands::Run {
            script,
            input,
            path,
        }) => run_run(script, input, path),
        Some(Commands::Repl { input, path }) => run_repl(input, path),
        Some(Commands::Watch {
            script,
            input,
            path,
            pick_inputs,
        }) => run_watch(script, input, path, pick_inputs),
        Some(Commands::History) => run_history(),
        Some(other) => {
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

    /// Same echo-protocol fake REPL used in repl.rs's/watch.rs's tests.
    fn fake_repl_config() -> repl::ReplConfig {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "blazewvr_main_fake_repl_{}_{id}.sh",
            std::process::id()
        ));
        let script = "#!/bin/sh\nprintf 'FAKE REPL\\n>>> '\nwhile IFS= read -r line; do\n  printf 'ECHO:%s\\n>>> ' \"$line\"\ndone\n";
        fs::write(&path, script).expect("write fake repl script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        repl::ReplConfig {
            program: PathBuf::from("/bin/sh"),
            args: vec![path.display().to_string()],
        }
    }

    /// A config whose program doesn't exist, so any `eval` fails.
    fn fake_repl_config_that_errors() -> repl::ReplConfig {
        repl::ReplConfig {
            program: PathBuf::from("/nonexistent/blazewvr/nope"),
            args: vec![],
        }
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
            vec!["=bad-input".into()],
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
    fn run_run_rejects_missing_script_file() {
        let dir = test_dir();
        let missing = dir.join("nope.dwl");
        let result = run_run(missing.display().to_string(), vec![], None);
        assert!(result.is_err());
    }

    #[test]
    fn prepare_run_rejects_missing_script_file() {
        let dir = test_dir();
        let missing = dir.join("nope.dwl");
        let result = prepare_run(&dir, &missing.display().to_string(), vec![], None);
        assert!(result.is_err());
    }

    #[test]
    fn prepare_run_rejects_invalid_input_format() {
        let dir = test_dir();
        let script = dir.join("s.dwl");
        fs::write(&script, "payload").unwrap();
        let result = prepare_run(
            &dir,
            &script.display().to_string(),
            vec!["=bad-input".into()],
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn prepare_run_reads_script_and_resolves_sidecar_inputs() {
        let dir = test_dir();
        let script = dir.join("s.dwl");
        fs::write(&script, "payload.orders").unwrap();
        fs::write(dir.join("s.payload.json"), "{}").unwrap();

        let plan = prepare_run(&dir, &script.display().to_string(), vec![], None).unwrap();
        assert_eq!(plan.src, "payload.orders");
        assert_eq!(
            plan.inputs,
            vec![("payload".to_string(), dir.join("s.payload.json"))]
        );
        assert_eq!(plan.module_path, None);
    }

    #[test]
    fn prepare_run_uses_config_inputs_and_module_path_when_no_cli_overrides() {
        let dir = test_dir();
        let script = dir.join("s.dwl");
        fs::write(&script, "payload").unwrap();
        fs::write(dir.join("blazewvr.toml"), "module_path = \"libs\"\n").unwrap();

        let plan = prepare_run(&dir, &script.display().to_string(), vec![], None).unwrap();
        assert_eq!(plan.module_path, Some("libs".to_string()));
    }

    #[test]
    fn prepare_run_cli_path_wins_over_config() {
        let dir = test_dir();
        let script = dir.join("s.dwl");
        fs::write(&script, "payload").unwrap();
        fs::write(dir.join("blazewvr.toml"), "module_path = \"libs\"\n").unwrap();

        let plan = prepare_run(
            &dir,
            &script.display().to_string(),
            vec![],
            Some("cli-dir".into()),
        )
        .unwrap();
        assert_eq!(plan.module_path, Some("cli-dir".to_string()));
    }

    #[test]
    fn repl_eval_line_colorizes_successful_output() {
        let mut sup = repl::Supervisor::new(fake_repl_config());
        let out = colorize::strip_ansi(&repl_eval_line(&mut sup, "hello"));
        assert!(out.contains("ECHO:hello"));
    }

    #[test]
    fn repl_eval_line_highlights_errors() {
        let mut sup = repl::Supervisor::new(fake_repl_config_that_errors());
        let out = repl_eval_line(&mut sup, "boom");
        assert!(out.contains("error"));
    }

    #[test]
    fn cli_parses_path_flag() {
        let cli = Cli::parse_from(["blazewvr", "run", "s.dwl", "--path", "a:b"]);
        match cli.command {
            Some(Commands::Run { path, .. }) => assert_eq!(path, Some("a:b".to_string())),
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
            vec!["=bad-input".into()],
            None,
            false,
        );
        assert!(result.is_err());
    }

    #[test]
    fn cli_parses_watch_subcommand() {
        let cli = Cli::parse_from(["blazewvr", "watch", "s.dwl", "-i", "a=b.json"]);
        match cli.command {
            Some(Commands::Watch { script, input, .. }) => {
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
            Some(Commands::Watch { script, .. }) => assert_eq!(script, None),
            _ => panic!("expected Watch"),
        }
    }

    #[test]
    fn cli_parses_watch_pick_inputs_flag() {
        let cli = Cli::parse_from(["blazewvr", "watch", "s.dwl", "--pick-inputs"]);
        match cli.command {
            Some(Commands::Watch { pick_inputs, .. }) => assert!(pick_inputs),
            _ => panic!("expected Watch"),
        }
    }

    #[test]
    fn cli_parses_history_subcommand() {
        let cli = Cli::parse_from(["blazewvr", "history"]);
        assert!(matches!(cli.command, Some(Commands::History)));
    }

    #[test]
    fn cli_parses_bare_invocation_as_no_command() {
        let cli = Cli::parse_from(["blazewvr"]);
        assert!(cli.command.is_none());
    }

    #[test]
    fn cli_parses_repl_input_flag() {
        let cli = Cli::parse_from(["blazewvr", "repl", "-i", "payload=data.json"]);
        match cli.command {
            Some(Commands::Repl { input, .. }) => {
                assert_eq!(input, vec!["payload=data.json".to_string()])
            }
            _ => panic!("expected Repl"),
        }
    }

    #[test]
    fn run_repl_rejects_invalid_input_format() {
        let result = run_repl(vec!["=bad-input".into()], None);
        assert!(result.is_err());
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
            Some(Commands::Run { script, input, .. }) => {
                assert_eq!(script, "script.dwl");
                assert_eq!(input, vec!["a.json".to_string()]);
            }
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn cli_parses_bare_script_with_no_subcommand() {
        let cli = Cli::parse_from(["blazewvr", "script.dwl", "-i", "a.json"]);
        assert!(cli.command.is_none());
        assert_eq!(cli.script, Some("script.dwl".to_string()));
        assert_eq!(cli.input, vec!["a.json".to_string()]);
    }

    fn spans_text(spans: &[span::StyledSpan]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn static_playground_state_shows_a_placeholder_dwl_script_and_no_input() {
        let state = static_playground_state();
        assert!(spans_text(&state.script_spans).contains("%dw 2.0"));
        assert_eq!(state.inputs.len(), 1);
        assert!(state.inputs[0].0.contains("no input bound"));
        assert!(state.inputs[0].1.is_empty());
        assert!(state.output_spans.is_empty());
    }

    #[test]
    fn playground_inputs_display_shows_a_single_placeholder_tab_when_none_bound() {
        let tabs = playground_inputs_display(&[]);
        assert_eq!(tabs.len(), 1);
        assert!(tabs[0].0.contains("no input bound"));
        assert!(tabs[0].1.is_empty());
    }

    #[test]
    fn playground_inputs_display_reads_and_labels_each_bound_input() {
        let dir = test_dir();
        let payload = dir.join("payload.json");
        fs::write(&payload, r#"{"a": 1}"#).unwrap();
        let headers = dir.join("headers.json");
        fs::write(&headers, r#"{"b": 2}"#).unwrap();

        let tabs = playground_inputs_display(&[
            ("payload".to_string(), payload),
            ("headers".to_string(), headers),
        ]);

        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].0, "payload");
        assert_eq!(spans_text(&tabs[0].1), r#"{"a": 1}"#);
        assert_eq!(tabs[1].0, "headers");
        assert_eq!(spans_text(&tabs[1].1), r#"{"b": 2}"#);
    }

    #[test]
    fn playground_inputs_display_reports_a_read_error_inline() {
        let missing = test_dir().join("missing.json");
        let tabs = playground_inputs_display(&[("payload".to_string(), missing)]);
        assert_eq!(tabs[0].0, "payload");
        assert!(spans_text(&tabs[0].1).contains("error reading"));
    }

    #[test]
    fn build_playground_state_reflects_current_script_and_output() {
        let dir = test_dir();
        let script = dir.join("s.dwl");
        fs::write(&script, "hello").unwrap();
        let target = watch::WatchTarget {
            script,
            inputs: vec![],
        };
        let mut sup = repl::Supervisor::new(fake_repl_config());

        let state = build_playground_state(&mut sup, &target);

        assert_eq!(spans_text(&state.script_spans), "hello");
        assert!(spans_text(&state.output_spans).contains("ECHO:hello"));
        assert!(state.inputs[0].0.contains("no input bound"));
    }

    #[test]
    fn build_playground_state_highlights_eval_errors() {
        let dir = test_dir();
        let script = dir.join("s.dwl");
        fs::write(&script, "boom").unwrap();
        let target = watch::WatchTarget {
            script,
            inputs: vec![],
        };
        let mut sup = repl::Supervisor::new(fake_repl_config_that_errors());

        let state = build_playground_state(&mut sup, &target);

        assert!(spans_text(&state.output_spans).contains("error"));
    }

    #[test]
    fn run_playground_ticks_quits_on_q() {
        let dir = test_dir();
        let script = dir.join("s.dwl");
        fs::write(&script, "one").unwrap();
        let target = watch::WatchTarget {
            script,
            inputs: vec![],
        };
        let mut sup = repl::Supervisor::new(fake_repl_config());

        let mut draws = 0;
        let mut calls = 0;
        let result = run_playground_ticks(
            &mut sup,
            &target,
            |_state| {
                draws += 1;
                Ok(())
            },
            |_timeout| {
                calls += 1;
                if calls == 1 {
                    Ok(None)
                } else {
                    Ok(Some(crossterm::event::KeyCode::Char('q')))
                }
            },
        );

        assert!(result.is_ok());
        assert_eq!(draws, 2);
    }

    #[test]
    fn run_playground_ticks_quits_on_esc() {
        let dir = test_dir();
        let script = dir.join("s.dwl");
        fs::write(&script, "one").unwrap();
        let target = watch::WatchTarget {
            script,
            inputs: vec![],
        };
        let mut sup = repl::Supervisor::new(fake_repl_config());

        let result = run_playground_ticks(
            &mut sup,
            &target,
            |_state| Ok(()),
            |_timeout| Ok(Some(crossterm::event::KeyCode::Esc)),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn run_playground_ticks_ignores_non_quit_keys_and_keeps_looping() {
        let dir = test_dir();
        let script = dir.join("s.dwl");
        fs::write(&script, "one").unwrap();
        let target = watch::WatchTarget {
            script,
            inputs: vec![],
        };
        let mut sup = repl::Supervisor::new(fake_repl_config());

        let mut draws = 0;
        let mut calls = 0;
        run_playground_ticks(
            &mut sup,
            &target,
            |_state| {
                draws += 1;
                Ok(())
            },
            |_timeout| {
                calls += 1;
                if calls < 3 {
                    Ok(Some(crossterm::event::KeyCode::Char('x')))
                } else {
                    Ok(Some(crossterm::event::KeyCode::Char('q')))
                }
            },
        )
        .unwrap();

        assert_eq!(draws, 3);
    }

    #[test]
    fn run_playground_ticks_reevaluates_on_a_watched_file_change() {
        let dir = test_dir();
        let script = dir.join("s.dwl");
        fs::write(&script, "one").unwrap();
        let target = watch::WatchTarget {
            script: script.clone(),
            inputs: vec![],
        };
        let mut sup = repl::Supervisor::new(fake_repl_config());

        let mut outputs: Vec<String> = Vec::new();
        let mut calls = 0;
        run_playground_ticks(
            &mut sup,
            &target,
            |state| {
                outputs.push(spans_text(&state.output_spans));
                Ok(())
            },
            |_timeout| {
                calls += 1;
                match calls {
                    1 => {
                        std::thread::sleep(Duration::from_millis(20));
                        fs::write(&script, "two").unwrap();
                        Ok(None)
                    }
                    _ => Ok(Some(crossterm::event::KeyCode::Esc)),
                }
            },
        )
        .unwrap();

        assert!(outputs[0].contains("ECHO:one"));
        assert!(outputs.last().unwrap().contains("ECHO:two"));
    }

    #[test]
    fn run_playground_ticks_cycles_active_input_forward_and_wraps() {
        let dir = test_dir();
        let script = dir.join("s.dwl");
        fs::write(&script, "one").unwrap();
        let a = dir.join("a.json");
        fs::write(&a, "a").unwrap();
        let b = dir.join("b.json");
        fs::write(&b, "b").unwrap();
        let target = watch::WatchTarget {
            script,
            inputs: vec![("a".to_string(), a), ("b".to_string(), b)],
        };
        let mut sup = repl::Supervisor::new(fake_repl_config());

        let mut active_seen: Vec<usize> = Vec::new();
        let mut calls = 0;
        run_playground_ticks(
            &mut sup,
            &target,
            |state| {
                active_seen.push(state.active_input);
                Ok(())
            },
            |_timeout| {
                calls += 1;
                match calls {
                    1 | 2 => Ok(Some(crossterm::event::KeyCode::Tab)),
                    _ => Ok(Some(crossterm::event::KeyCode::Char('q'))),
                }
            },
        )
        .unwrap();

        // starts at 0, Tab -> 1, Tab -> wraps back to 0
        assert_eq!(active_seen, vec![0, 1, 0]);
    }

    #[test]
    fn run_playground_ticks_cycles_active_input_backward_and_wraps() {
        let dir = test_dir();
        let script = dir.join("s.dwl");
        fs::write(&script, "one").unwrap();
        let a = dir.join("a.json");
        fs::write(&a, "a").unwrap();
        let b = dir.join("b.json");
        fs::write(&b, "b").unwrap();
        let target = watch::WatchTarget {
            script,
            inputs: vec![("a".to_string(), a), ("b".to_string(), b)],
        };
        let mut sup = repl::Supervisor::new(fake_repl_config());

        let mut active_seen: Vec<usize> = Vec::new();
        let mut calls = 0;
        run_playground_ticks(
            &mut sup,
            &target,
            |state| {
                active_seen.push(state.active_input);
                Ok(())
            },
            |_timeout| {
                calls += 1;
                match calls {
                    1 => Ok(Some(crossterm::event::KeyCode::BackTab)),
                    _ => Ok(Some(crossterm::event::KeyCode::Char('q'))),
                }
            },
        )
        .unwrap();

        // starts at 0, Shift+Tab wraps back to the last tab
        assert_eq!(active_seen, vec![0, 1]);
    }
}
