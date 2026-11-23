mod math;

use crate::PolarsPlugin;
use console_plugin::PluginCommand;

pub(crate) fn computation_commands() -> Vec<Box<dyn PluginCommand<Plugin = PolarsPlugin>>> {
    math::commands()
}
