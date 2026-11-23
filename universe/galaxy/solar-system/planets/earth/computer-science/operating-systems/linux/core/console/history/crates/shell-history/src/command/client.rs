use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use clap::Subcommand;
use eyre::{Result, WrapErr};

use shell_history_client::{
    database::Sqlite, record::sqlite_store::SqliteStore, settings::Settings,
};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{
    Layer, filter::EnvFilter, filter::LevelFilter, fmt, fmt::format::FmtSpan, prelude::*,
};

fn cleanup_old_logs(log_dir: &Path, prefix: &str, retention_days: u64) {
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(retention_days * 24 * 60 * 60);

    let Ok(entries) = fs::read_dir(log_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        // Match files like "search.log.2024-02-23" or "daemon.log.2024-02-23"
        if !name.starts_with(prefix) || name == prefix {
            continue;
        }

        if let Ok(metadata) = entry.metadata()
            && let Ok(modified) = metadata.modified()
            && modified < cutoff
        {
            let _ = fs::remove_file(&path);
        }
    }
}

#[cfg(feature = "sync")]
mod sync;

#[cfg(feature = "sync")]
mod account;


mod config;
mod default_config;
mod doctor;
mod ensure;
mod server;
mod history;
mod import;
mod info;
mod init;
mod search;
mod stats;
mod store;

#[derive(Subcommand, Debug)]
#[command(infer_subcommands = true)]
pub enum Cmd {

    /// Manipulate shell history
    #[command(subcommand)]
    History(history::Cmd),


    /// Import shell history from file
    #[command(subcommand)]
    Import(import::Cmd),

    /// Calculate statistics for your history
    Stats(stats::Cmd),

    /// Interactive history search
    Search(search::Cmd),

    #[cfg(feature = "sync")]
    #[command(flatten)]
    Sync(sync::Cmd),

    /// Manage your sync account
    #[cfg(feature = "sync")]
    Account(account::Cmd),


    /// Manage the console history data store
    #[command(subcommand)]
    Store(store::Cmd),



    /// Print console history's shell init script
    #[command()]
    Init(init::Cmd),

    /// Information about dotfiles locations and ENV vars
    #[command()]
    Info,

    /// Run the doctor to check for common issues
    #[command()]
    Doctor,



    /// Print the default console history configuration (config.toml)
    #[command()]
    DefaultConfig,
    /// Start the local history server
    #[command()]
    Server,

    /// Start the local server and register if needed
    #[command()]
    Ensure,

    #[command(subcommand)]
    Config(config::Cmd),


}

impl Cmd {
    pub fn run(self) -> Result<()> {


        let mut runtime = tokio::runtime::Builder::new_current_thread();

        let runtime = runtime.enable_all().build().unwrap();

        // For non-history commands, we want to initialize logging before
        // doing anything else. History commands are performance-sensitive and run before and after
        // every shell command, so we want to skip any unnecessary initialization for them.
        let settings = Settings::new().wrap_err("could not load client settings")?;
        let res = runtime.block_on(self.run_inner(settings));

        runtime.shutdown_timeout(std::time::Duration::from_millis(50));

        res
    }

    #[allow(clippy::too_many_lines, clippy::future_not_send)]
    async fn run_inner(
        self,
        mut settings: Settings,
    ) -> Result<()> {
        // CONSOLE_SHELL_HISTORY_LOG env var overrides config file level settings
        let env_log_set = std::env::var("CONSOLE_SHELL_HISTORY_LOG").is_ok();

        // Base filter from env var (or empty if not set)
        let base_filter =
            EnvFilter::from_env("CONSOLE_SHELL_HISTORY_LOG").add_directive("sqlx_sqlite::regexp=off".parse()?);

        let is_interactive_search = matches!(&self, Self::Search(cmd) if cmd.is_interactive());
        // Use file-based logging for interactive search (TUI mode)
        let use_search_logging = is_interactive_search && settings.logs.search_enabled();


        // Set up span timing JSON logs if CONSOLE_SHELL_HISTORY_SPAN is set
        let span_path = std::env::var("CONSOLE_SHELL_HISTORY_SPAN").ok().map(|p| {
            if p.is_empty() {
                "shell-history-spans.json".to_string()
            } else {
                p
            }
        });

        // Helper to create span timing layer
        macro_rules! make_span_layer {
            ($path:expr) => {{
                let span_file = OpenOptions::new()
                    .create(true)
                    .truncate(true)
                    .write(true)
                    .open($path)?;
                Some(
                    fmt::layer()
                        .json()
                        .with_writer(span_file)
                        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
                        .with_filter(LevelFilter::TRACE),
                )
            }};
        }

        // Build the subscriber with all configured layers
        if use_search_logging {
            let search_filename = settings.logs.search.file.clone();
            let log_dir = PathBuf::from(&settings.logs.dir);
            fs::create_dir_all(&log_dir)?;

            // Clean up old log files
            cleanup_old_logs(&log_dir, &search_filename, settings.logs.search_retention());

            let file_appender =
                RollingFileAppender::new(Rotation::DAILY, &log_dir, &search_filename);

            // Use config level unless CONSOLE_SHELL_HISTORY_LOG is set
            let filter = if env_log_set {
                base_filter
            } else {
                EnvFilter::default()
                    .add_directive(settings.logs.search_level().as_directive().parse()?)
                    .add_directive("sqlx_sqlite::regexp=off".parse()?)
            };

            let base = tracing_subscriber::registry().with(
                fmt::layer()
                    .with_writer(file_appender)
                    .with_ansi(false)
                    .with_filter(filter),
            );

            match &span_path {
                Some(sp) => {
                    base.with(make_span_layer!(sp)).init();
                }
                None => {
                    base.init();
                }
            }
        }

        tracing::trace!(command = ?self, "client command");

        // Skip initializing any databases for history
        // This is a pretty hot path, as it runs before and after every single command the user
        // runs
        match self {
            Self::History(history) => return history.run(&settings).await,
            Self::Init(init) => return init.run(&settings).await,
            Self::Doctor => return doctor::run(&settings).await,
            Self::Server => return server::run().await,
            Self::Ensure => return ensure::run(&settings).await,
            Self::Config(config) => return config.run(&settings).await,
            _ => {}
        }

        let db_path = PathBuf::from(settings.db_path.as_str());
        let record_store_path = PathBuf::from(settings.record_store_path.as_str());

        let db = Sqlite::new(db_path, settings.local_timeout).await?;
        let sqlite_store = SqliteStore::new(record_store_path, settings.local_timeout).await?;

        match self {
            Self::Import(import) => import.run(&db).await,
            Self::Stats(stats) => stats.run(&db, &settings).await,
            Self::Search(search) => search.run(db, &mut settings, sqlite_store).await,

            #[cfg(feature = "sync")]
            Self::Sync(sync) => sync.run(settings, &db, sqlite_store).await,

            #[cfg(feature = "sync")]
            Self::Account(account) => account.run(settings, sqlite_store).await,


            Self::Store(store) => store.run(&settings, &db, sqlite_store).await,



            Self::Info => {
                info::run(&settings);
                Ok(())
            }

            Self::DefaultConfig => {
                default_config::run();
                Ok(())
            }



            Self::History(_) | Self::Init(_) | Self::Doctor | Self::Config(_) | Self::Server | Self::Ensure => {
                unreachable!()
            }


        }
    }
}
