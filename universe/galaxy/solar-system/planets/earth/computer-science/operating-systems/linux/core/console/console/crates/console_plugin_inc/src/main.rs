use console_plugin::{JsonSerializer, serve_plugin};
use console_plugin_inc::IncPlugin;

fn main() {
    serve_plugin(&IncPlugin, JsonSerializer {})
}
