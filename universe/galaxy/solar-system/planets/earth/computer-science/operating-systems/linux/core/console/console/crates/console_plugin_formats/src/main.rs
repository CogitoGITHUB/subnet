use console_plugin::{MsgPackSerializer, serve_plugin};
use console_plugin_formats::FormatCmdsPlugin;

fn main() {
    serve_plugin(&FormatCmdsPlugin, MsgPackSerializer {})
}
