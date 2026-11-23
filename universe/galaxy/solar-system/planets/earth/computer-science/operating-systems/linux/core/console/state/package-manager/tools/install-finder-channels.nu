#!/usr/bin/env nu
# install-finder-channels.nu -- PRINTS the commands that install state's
# finder channels. It never writes anything itself (D-039: the finder
# side is opt-in, and a script that silently edits a config directory
# is exactly the surprise this project does not do).
#
# Run it, read the commands, run them yourself if you agree.

def main [
    --cwd: string
    --config: string
] {
    let cwd = (if ($cwd == null) { pwd | path expand } else { $cwd })
    let config = (if ($config == null) {
        $nu.home-dir | path join ".config" "finder"
    } else {
        $config
    })
    let src = ($cwd | path join "finder")
    let cable = ($config | path join "cable")

    print $"channel source: ($src)"
    print $"cable target: ($cable)"
    print ""
    print "Prerequisites (once):"
    print "  1. finder installed: state installs it"
    print "     (see docs/examples/finder.rktd, pinned v0.4.21)"
    print "  2. shell integration, save this line once:"
    print "     finder init nu | save -f vendor-autoload-finder.nu"
    print ""
    print "This script writes nothing. Run these yourself if you agree:"
    print $"  mkdir -p ($cable)"
    let specs_ln = ($src | path join 'state-specs.toml' | path expand)
    let packs_ln = ($src | path join 'state-packages.toml' | path expand)
    print $"  ln -sfn ($specs_ln) ($cable | path join 'state-specs.toml')"
    print $"  ln -sfn ($packs_ln) ($cable | path join 'state-packages.toml')"
    print ""
    print "Required environment (export before launching finder):"
    print $"  PM_ROOT=($cwd)  # the PM checkout"
    print "  PM_PROFILE=<profile dir> PM_LOCK=<lockfile>  # for state-packages"
    print "  PM_SPECS=<specs dir>  # optional, defaults to the repo examples"
    print ""
    print "Then list and launch:"
    print "  finder list-channels"
    print "  finder state-specs"
    print ""
    print "To remove again:"
    print $"  rm -f ($cable | path join 'state-specs.toml') ($cable | path join 'state-packages.toml')"
}

