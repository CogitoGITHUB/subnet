use eyre::Result;

pub async fn run() -> Result<()> {
    let dir = shell_history_common::utils::data_dir();
    std::fs::create_dir_all(&dir)?;
    let db = dir.join("server.db");
    if !db.exists() {
        std::fs::File::create(&db)?;
    }
    let settings = shell_history_server::Settings::embedded(format!(
        "sqlite://{}",
        db.display()
    ));
    let addr: std::net::SocketAddr = "127.0.0.1:8888".parse()?;
    shell_history_server::launch::<shell_history_server_sqlite::Sqlite>(settings, addr).await
}
