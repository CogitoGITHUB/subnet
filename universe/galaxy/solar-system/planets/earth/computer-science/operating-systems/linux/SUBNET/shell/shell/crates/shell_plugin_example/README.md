# Plugin Example

Crate with a simple example of the Plugin trait that needs to be implemented
in order to create a binary that can be registered into shell declaration list

## `example config`

This subcommand demonstrates sending configuration from the shell `$env.config` to a plugin.

To make use of the plugin after building `shell` run:

```shell
plugin add target/debug/shell_plugin_example
# or then either restart your current shell session or run:
plugin use target/debug/shell_plugin_example
```

The configuration for the plugin lives in `$env.config.plugins.example`:

```shell
$env.config = {
  plugins: {
    example: [
      some
      values
    ]
  }
}
```

To list plugin values run:

```shell
example config
```

