use shell_test_support::prelude::*;

#[test]
fn print_config_env_default_to_stdout() -> Result {
    test()
        .run("config env --default")
        .expect_value_eq(shell_config::ConfigFileKind::Env.default())
}
