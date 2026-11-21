R=$(cat $HOME/.local/state/subnet/root)
cd "$R"
export LANG=C.UTF-8 LC_ALL=C.UTF-8 PLTCOMPILEDROOTS=$HOME/run-compiled
exec $HOME/opt/racket-full/bin/racket "subnet/subnet TUI configuration/settings-tui.rkt"
