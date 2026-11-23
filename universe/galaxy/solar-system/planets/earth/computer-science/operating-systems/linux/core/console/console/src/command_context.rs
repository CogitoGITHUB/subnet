use console_protocol::engine::EngineState;

pub(crate) fn add_command_context(engine_state: EngineState) -> EngineState {
    let engine_state = console_cmd_lang::add_default_context(engine_state);
    #[cfg(feature = "plugin")]
    let engine_state = console_cmd_plugin::add_plugin_command_context(engine_state);
    let engine_state = console_command::add_console_command_context(engine_state);
    let engine_state = console_cmd_extra::add_extra_command_context(engine_state);
    let engine_state = console_cli::add_cli_context(engine_state);
    let engine_state = console_explore::add_explore_context(engine_state);
    console_tui::add_tui_context(engine_state)
}

