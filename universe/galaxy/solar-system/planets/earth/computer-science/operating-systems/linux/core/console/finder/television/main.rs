fn main() -> anyhow::Result<()> {
    television::cli_main::run(std::env::args_os())
}
