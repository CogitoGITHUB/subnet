fn main() -> eyre::Result<()> {
    shell_history::run(std::env::args_os().collect())
}
