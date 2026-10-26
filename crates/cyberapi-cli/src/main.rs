//! A4: `cyberapi-cli run FILE`: a project's APIs, or one folder's, sent one
//! after another in the tree's order as the app's Run… sends them, for a
//! terminal or CI. FILE is whatever the app imports — CyberAPI's own export
//! (kept in git, its ids kept for `get`), a Postman collection, Insomnia,
//! OpenAPI, Bruno, Hoppscotch… — held in memory: no database, no server.
//! `--env` picks one of its named environments, `--env-file` adds another
//! file's, `--var` and `--data` give values read as `{{value(name)}}`.
//! `env(NAME)` reads this process's environment, CI's secrets. Each send is
//! printed as it ends; `--junit` writes the run as JUnit XML. The exit code
//! is 0 when every send held, 1 when one did not, 2 when nothing could run.

mod report;

use clap::{Parser, Subcommand};
use cyberapi_core::import::{self, ImportBundle};
use cyberapi_core::inherit;
use cyberapi_core::prefs::{SendPrefs, Timeouts};
use cyberapi_core::run::{self as rules, Values};
use cyberapi_core::sync::ProjectConfig;
use cyberapi_core::template::{Answers, BaseDir, Env, EnvReader, FileReader};
use cyberapi_core::tree::{ItemKind, Tree};
use cyberapi_core::{ItemId, version};
use cyberapi_http::{NativeEngine, TargetPolicy};
use cyberapi_service::{RunStep, Sender, StepResult};
use cyberapi_store::{MemoryStore, NewCollection};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// What `--version` says: the release's version or a nightly's, and the
/// commit.
fn described() -> &'static str {
    static DESCRIBED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    DESCRIBED.get_or_init(version::describe)
}

#[derive(Parser)]
#[command(name = "cyberapi-cli", version = described(), about = "CyberAPI without its window")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Runs a project file's APIs, or one folder's, in the tree's order.
    Run(RunArgs),
}

#[derive(clap::Args)]
struct RunArgs {
    /// The project: CyberAPI's own export, a Postman collection, an
    /// Insomnia export, an OpenAPI document, a Bruno or Hoppscotch file…
    file: PathBuf,
    /// Runs only the folder of this path (`users/admin`) or id.
    #[arg(long)]
    folder: Option<String>,
    /// The named environment to run in, by its name in any case.
    #[arg(long)]
    env: Option<String>,
    /// A file whose variables and named environments are added: a Postman
    /// or Hoppscotch environment, a Bruno environment… Later ones stand
    /// for earlier ones; with no --env, the last one's environment, when it
    /// has one, is the run's.
    #[arg(long = "env-file", value_name = "FILE")]
    env_files: Vec<PathBuf>,
    /// A value for every request, read as {{value(NAME)}} and as an
    /// address's {{NAME}}, over the project's of that name.
    #[arg(long = "var", value_name = "NAME=VALUE", value_parser = pair)]
    vars: Vec<(String, String)>,
    /// A data file — CSV with a header row, or a JSON array of objects —
    /// run once for each row, its values read as --var's are.
    #[arg(long, value_name = "FILE")]
    data: Option<PathBuf>,
    /// What a prompt(LABEL) is answered with.
    #[arg(long = "prompt", value_name = "LABEL=VALUE", value_parser = pair)]
    prompts: Vec<(String, String)>,
    /// Writes the run as JUnit XML to this file.
    #[arg(long, value_name = "FILE")]
    junit: Option<PathBuf>,
    /// Follows redirects, as the app's setting does.
    #[arg(long)]
    follow_redirects: bool,
    /// Accepts any certificate: self-signed, expired, another name's.
    #[arg(long)]
    insecure: bool,
}

/// `NAME=VALUE`.
fn pair(text: &str) -> Result<(String, String), String> {
    match text.split_once('=') {
        Some((name, value)) if !name.trim().is_empty() => Ok((name.trim().to_string(), value.to_string())),
        _ => Err(format!("{text:?} is not NAME=VALUE")),
    }
}

/// Files a request reads (`readFile`): one named by a path relative to the
/// project file, or a whole one.
struct Files {
    base: PathBuf,
}

impl FileReader for Files {
    fn read(&self, file: &str, _: Option<BaseDir>) -> Result<Vec<u8>, String> {
        let path = self.base.join(file);
        std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))
    }
}

/// `env(NAME)` (D2): this process's environment — CI's secrets.
struct ProcessEnv;

impl EnvReader for ProcessEnv {
    fn read(&self, names: &[String]) -> Env {
        Env::Read(
            names
                .iter()
                .map(|name| (name.clone(), std::env::var(name).ok()))
                .collect(),
        )
    }
}

fn main() -> ExitCode {
    let Command::Run(args) = Cli::parse().command;
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(2);
        }
    };
    let handle = runtime.handle().clone();
    match runtime.block_on(run(args, handle)) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(2)
        }
    }
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
}

/// The folder `wanted` names: its id, or its path of names from the top.
fn folder(tree: &Tree, wanted: &str) -> Option<ItemId> {
    let id = ItemId::new(wanted);
    if tree.get(&id).is_some_and(|node| node.kind() == ItemKind::Folder) {
        return Some(id);
    }
    let mut level = tree.top_level_ids();
    let mut found = None;
    for name in wanted.split('/').map(str::trim).filter(|name| !name.is_empty()) {
        let next = level.iter().find(|id| {
            tree.get(id)
                .is_some_and(|node| node.kind() == ItemKind::Folder && node.name() == name)
        })?;
        found = Some(next.clone());
        level = tree.get(next).map(|node| node.children().to_vec()).unwrap_or_default();
    }
    found
}

/// Runs it; whether every send held, or why it could not run.
async fn run(args: RunArgs, handle: tokio::runtime::Handle) -> Result<bool, String> {
    let text = read(&args.file)?;
    let mut config: ProjectConfig =
        rules::project(&text).map_err(|error| format!("{}: {error}", args.file.display()))?;
    // CyberAPI's own export says no name: the file's.
    if config.name.trim().is_empty() {
        config.name = args
            .file
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "CyberAPI".into());
    }
    // An environment file's own environment, the last one's.
    let mut from_file = None;
    for file in &args.env_files {
        let bundle: ImportBundle =
            import::parse(&read(file)?).map_err(|error| format!("{}: {error}", file.display()))?;
        rules::add_environments(&mut config, &bundle.environment, &bundle.environments);
        if let [environment] = &bundle.environments[..] {
            from_file = Some(environment.name.clone());
        }
    }
    let tree_data = config.tree();
    let tree = Tree::build(&tree_data);
    let within = match &args.folder {
        Some(wanted) => Some(folder(&tree, wanted).ok_or_else(|| format!("no folder {wanted:?} in the project"))?),
        None => None,
    };
    let environment = match args.env.as_ref().or(from_file.as_ref()) {
        Some(wanted) => Some(
            config
                .environments
                .iter()
                .find(|environment| environment.name.eq_ignore_ascii_case(wanted.trim()))
                .cloned()
                .ok_or_else(|| {
                    let named: Vec<&str> = config.environments.iter().map(|e| e.name.as_str()).collect();
                    format!("no environment {wanted:?}; the project has {}", report::listed(&named))
                })?,
        ),
        None => None,
    };
    let rows: Vec<Values> = match &args.data {
        Some(file) => rules::rows(&read(file)?).map_err(|error| format!("{}: {error}", file.display()))?,
        None => Vec::new(),
    };
    // --var under each row's values, which stand for it.
    let rows: Vec<Values> = if rows.is_empty() {
        if args.vars.is_empty() {
            Vec::new()
        } else {
            vec![args.vars.clone()]
        }
    } else {
        rows.into_iter()
            .map(|row| {
                let mut values = args.vars.clone();
                for (name, value) in row {
                    rules::set(&mut values, &name, value);
                }
                values
            })
            .collect()
    };
    let passes = rows.len().max(1);

    let storage = MemoryStore::new().storage();
    let collection = storage
        .collections()
        .create(NewCollection {
            name: config.name.clone(),
            ..Default::default()
        })
        .await
        .map_err(|error| error.to_string())?;
    storage
        .tree()
        .replace(collection.id.clone(), config.clone(), None)
        .await
        .map_err(|error| error.to_string())?;

    let planned = rules::plan(&tree_data, within.as_ref());
    if planned.is_empty() {
        return Err("nothing to run: no API there".into());
    }
    let steps: Vec<RunStep> = planned
        .iter()
        .filter_map(|id| tree_data.apis.iter().find(|api| &api.id == id))
        .map(|api| RunStep {
            api: api.id.clone(),
            request: api.request.clone(),
            inherited: inherit::inherited(&tree_data, &api.id),
        })
        .collect();
    let base = args
        .file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let sender = Sender {
        storage,
        engine: Arc::new(NativeEngine::new(handle, TargetPolicy::Anywhere).with_proxies()),
        files: Arc::new(Files { base }),
        env: Arc::new(ProcessEnv),
        user_agent: format!("CyberAPI/{} (cli)", version::VERSION),
        timeouts: Timeouts::default(),
        sending: SendPrefs {
            follow_redirects: args.follow_redirects,
            accept_invalid_certs: args.insecure,
            ..Default::default()
        },
        inherited: Default::default(),
        environment,
    };
    let places: HashMap<ItemId, report::Place> = steps
        .iter()
        .map(|step| (step.api.clone(), report::place(&tree, &step.api)))
        .collect();
    let named: Vec<report::Place> = steps.iter().map(|step| places[&step.api].clone()).collect();
    let started = Instant::now();
    let results: Arc<Mutex<Vec<StepResult>>> = Arc::default();
    let told = results.clone();
    let printed = named.clone();
    let steps_kept = steps.clone();
    cyberapi_service::run(
        sender,
        collection.id,
        steps.clone(),
        rows,
        Answers {
            file: None,
            prompts: args.prompts.into_iter().collect(),
        },
        Arc::new(move |result| {
            let place = &printed[result.step];
            println!("{}", report::line(&result, place, passes > 1));
            for detail in report::details(&result, &steps[result.step].request.assertions) {
                println!("    {detail}");
            }
            if let Ok(mut results) = told.lock() {
                results.push(result);
            }
        }),
    )
    .await;
    let results = results.lock().map(|results| results.clone()).unwrap_or_default();
    let held = results.iter().filter(|result| result.held()).count();
    println!();
    println!(
        "{held} held, {} failed, in {:.2} s",
        results.len() - held,
        started.elapsed().as_secs_f64()
    );
    if let Some(file) = &args.junit {
        let cases: Vec<rules::Case> = results
            .iter()
            .map(|result| {
                let assertions = &steps_kept[result.step].request.assertions;
                report::case(result, &named[result.step], passes > 1, assertions)
            })
            .collect();
        std::fs::write(file, rules::junit(&config.name, &cases))
            .map_err(|error| format!("{}: {error}", file.display()))?;
    }
    Ok(held == results.len())
}
