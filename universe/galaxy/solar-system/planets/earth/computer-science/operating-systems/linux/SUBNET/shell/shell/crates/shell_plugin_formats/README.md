# shell_plugin_formats
A shell plugin to convert data to shell tables.

# support commands:
1. from eml - original ported from shell core.
2. from ics - original ported from shell core.
3. from ini - original ported from shell core.
4. from vcf - original ported from shell core.
5. from plist - original ported from shell core.
6. to plist - original ported from shell core.

# Prerequisite
`shell`, It's a shell plugin, so you need it.

# Usage
1. compile the binary: `cargo build`
2. plugin add plugin(assume it's compiled in ./target/debug/):
```
plugin add ./target/debug/shell_plugin_formats
```
