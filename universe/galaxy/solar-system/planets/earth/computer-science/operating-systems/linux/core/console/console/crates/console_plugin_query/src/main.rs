use console_plugin::{JsonSerializer, serve_plugin};
use console_plugin_query::Query;

fn main() {
    serve_plugin(&Query {}, JsonSerializer {})
}
