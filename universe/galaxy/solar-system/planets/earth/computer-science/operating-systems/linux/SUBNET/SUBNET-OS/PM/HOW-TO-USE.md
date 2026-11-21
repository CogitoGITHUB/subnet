# RUN ecosystem: how to use it (the only way)

Everything is built and installed through the PM. Never run cargo, raco or a separate build by hand. Never symlink into /usr/local/bin.

## Commands (run from the ecosystem root)
- subnet status: what is installed
- subnet live NAME: fast incremental build, runs from the live build
- subnet dev NAME: full build through the PM and the server, rolls back on failure
- subnet update [--dry]: rebuild only what changed (fingerprints)
- ecosystem apply: apply the whole manifest
- subnet serve / stop: start or stop the shared server (shell starts it by itself)
- subnet devmode on|off|toggle|status: dev mode flag, off builds everything changed
- subnet devwatch: started by dev mode on, live-builds every installed package whose source changed (log ~/.local/state/subnet/devwatch.log)
- ecosystem mux / edit / editor / pm: multiplexer, editor, package manager

## Keys (shell screen, visual mode, also inside the multiplexer)
- m: dev mode on or off, off builds everything changed through the PM
- M: build everything changed now
- z: zoxide picker, Z: add this directory to zoxide

## Boxes in the multiplexer
- RUN server box at the top: running, stopped or not installed
- red DEV MODE ACTIVE box next to it while dev mode is on
- the shell screen shows the same boxes right above its own bar when it runs outside the multiplexer

## Never break
- builds write logs to files, never pipe a build to head
- state lives in ~/.local/state/subnet (dev-mode flag, fingerprints, devmode.log)

## Errors
- failed builds in dev mode write the compile text to ~/.local/state/subnet/dev-errors/NAME.log (a good build deletes it)
- a failed update also includes the last lines of the server build log (~/.local/state/subnet/dev.log)
- subnet deverrors: print all of them, subnet deverrors clear: wipe them
- the multiplexer dev box shows only the error count

## Launch line (when ecosystem is not on PATH)
Run from the ecosystem root, one command, log to a file:

export LANG=C.UTF-8 LC_ALL=C.UTF-8 PLTCOMPILEDROOTS=$HOME/run-compiled
$HOME/opt/racket-full/bin/racket "subnet/entry/subnet.rkt" live shell > /tmp/eco-live.log 2>&1

- Replace "live shell" with any command from the list above.
- Package name is shell (it includes shell-screen), not a crate name.
- Read the result with: tail -c 1200 /tmp/eco-live.log
