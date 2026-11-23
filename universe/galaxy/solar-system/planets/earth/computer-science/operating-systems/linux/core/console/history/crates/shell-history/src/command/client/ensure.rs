use std::net::TcpStream;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

use eyre::Result;
use shell_history_client::{api_client, encryption, settings::Settings};

const ADDR: &str = "127.0.0.1:8888";

fn up() -> bool {
    match ADDR.parse() {
        Ok(a) => TcpStream::connect_timeout(&a, Duration::from_millis(200)).is_ok(),
        Err(_) => false,
    }
}

/// Start the local server if it is down, then register once.
pub async fn run(settings: &Settings) -> Result<()> {
    if !up() {
        let me = std::env::current_exe()?;
        Command::new(me)
            .args(["history", "server"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        for _ in 0..50 {
            if up() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    if !up() {
        return Ok(());
    }
    let meta = Settings::meta_store().await?;
    if !meta.logged_in().await? {
        let reg = api_client::register(
            settings.sync_address.as_str(),
            "console",
            "console@localhost",
            "console-local",
        )
        .await;
        if let Ok(session) = reg {
            meta.save_session(&session.session).await?;
            let _key = encryption::load_key(settings)?;
        }
    }
    Ok(())
}
