use shell_plugin::{JsonSerializer, serve_plugin};
use shell_plugin_inc::IncPlugin;

fn main() {
    serve_plugin(&IncPlugin, JsonSerializer {})
}
