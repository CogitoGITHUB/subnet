use eyre::Result;

pub async fn run() -> Result<()> {
    shell_history_client::logout::logout().await
}
