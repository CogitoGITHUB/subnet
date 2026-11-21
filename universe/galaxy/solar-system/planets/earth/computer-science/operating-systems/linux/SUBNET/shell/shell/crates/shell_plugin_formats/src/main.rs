use shell_plugin::{MsgPackSerializer, serve_plugin};
use shell_plugin_formats::FormatCmdsPlugin;

fn main() {
    serve_plugin(&FormatCmdsPlugin, MsgPackSerializer {})
}
