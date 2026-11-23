use console_plugin::{MsgPackSerializer, serve_plugin};
use console_plugin_gstat::GStatPlugin;

fn main() {
    serve_plugin(&GStatPlugin, MsgPackSerializer {})
}
