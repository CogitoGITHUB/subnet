R=$(cat $HOME/.local/state/console/root)
cd "$R"
export LANG=C.UTF-8 LC_ALL=C.UTF-8 PLTCOMPILEDROOTS=$HOME/run-compiled
exec $HOME/opt/racket-full/bin/racket "entry/settings/settings-tui.rkt"
