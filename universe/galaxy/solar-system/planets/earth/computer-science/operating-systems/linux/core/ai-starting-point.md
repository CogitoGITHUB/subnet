RULE 0 (THEME): NEVER configure a theme in any program or
config file (no settings.toml, no finder theme, no generated
files, no sync scripts). Every program reads colors ONLY
from my theme engine (styles/active, via style.rkt in Racket
or a small module that reads the same files in Rust). One
file edit must recolor everything. Splash and finder already do
this (console/splash/src/theme/run.rs and
console/finder/television/config/themes/run.rs).

# START HERE (for any AI)

NEVER LEAVE OPEN HOLES. If you see a hole or a side effect in your own work, close it in the same script. Never write a known hole as a note to him.

This file only explains how to work on this system. It holds no work,
no progress and no next steps. The work is in the todo files and the
plan files. Read ACTIVE WORK below, then go to the files it names.

## ACTIVE WORK
Area: tree-file-manager first class in the shell.
Plan: todo.run, the paragraphs starting todo: tree-file-manager step.
Read them all, then do the first step left, in order.

When the area changes, replace only the lines inside this block.
Never add steps, results or status here. Never write what was done.

## The person
Shape works on a phone (Termux+shroot). He is angry about wasted
tokens. Small moves, small reads. Ask for ALL the code you need in
ONE read. Short answers, answer first, no preamble, ONE code block
per answer, lines under 70 columns. Never make him paste many
times. He may hand you only this file.

## Where and which shell
- Folder: the one this file is in (SUBNET). Start every script
  with cd to its absolute path, written as one line. Relative paths
  after that.
- Shell is shell, a Nushell fork. Wrap bash in bash -c "...".
- runtd on PATH is /usr/local/bin/runtd, not the PM launcher.
  It must export SUBNET_HOT_PATH_BIN and RUNFILE_BIN (prefix
  bins), else a stale hot-path runs and the first draw hangs.

## Rules
- Say what you started only when you start something
  new: one short line. Never recap it in later answers.
- No git, no backup files, no guessed paths, no find over home, no
  comments in code.
- Read first, then patch: grep -c every anchor, abort if the count is
  not the expected one, before every sed. Racket: load check after.
  Rust: the build is the check.
- Never cat big files: tr -d "\033\r" | fold -w 70 | head -c N.
- Output shown in a terminal must wrap to the screen width.
- OS: Alpine in shroot (musl). install.sh builds Racket from src.
- ALWAYS push everything after every change. Nothing behind.
- Racket only: $HOME/opt/racket-full/bin/racket with env
  LANG=C.UTF-8 LC_ALL=C.UTF-8 PLTCOMPILEDROOTS=$HOME/run-compiled
- Builds only through the PM, as in SUBNET-OS/PM/HOW-TO-USE.md
  (subnet live, dev, update, devmode). No cargo or raco by hand.
- BUILD (ecosystem is not on PATH in bash -c). From this folder:
  LANG=C.UTF-8 LC_ALL=C.UTF-8 PLTCOMPILEDROOTS=$HOME/run-compiled
  $HOME/opt/racket-full/bin/racket
  "subnet/entry/subnet.rkt" live NAME
  > /tmp/eco-live.log 2>&1 ; tail -c 400 /tmp/eco-live.log
  Join the lines. NAME = package. A runfile change needs
  runfile, then editor. Never read HOW-TO-USE.md for it.
- Do not touch the mux z key.
- tree-file-manager fork: console/tree-file-manager, shallow clone of v1.55.0, no .git. The tree-file-manager entry in shell/upstream-version.txt holds url, commit, version. Never run git inside it. My files (managed) change only through the server via shell/run-files; every other file works normally.
- READ ROOT-WARNING.md in this folder before touching file locks, permissions, the server write path or tree-file-manager. This system has no real root.
- OPEN THE EDITOR IN THE SAME CODE BLOCK, ONLY FOR RUNFILE FILES. Open only .run files, with runtd FILE.run, after the bash -c block inside the SAME code block, and only when the change touches runfile or the editor and a .run test page shows it. NEVER open .md, .rkt, .rs, .test or any other file, and never after a read-only step or an edit of a non-runfile file. Never a second block. If no test page fits a runfile change, make a small one in the same block.
- ALL CHANGES IN TMP FIRST. Every script copies each file it will change to /tmp, edits ONLY the /tmp copies, validates them (anchor counts, racket load, line counts), and only if EVERY step passed does it cp them over the real files, all at the end together. Any failure at any step: delete the tmp copies, touch no real file, print what failed. Never patch a real file in place and restore later: a half-applied change is the worst case. Builds run only after the copy-over, and a failed build restores from the saved originals.
- DOCS PAUSED: never run runtd docs and never touch EDITOR/
  runtd-tutorial.run until he says so. This overrides every rule
  here that says a capability must show in the tutorial or have
  a recorded test. Now only the book plan and the README matter.
- Every finished step of the book plan updates FILE/
  README.run in the same script, so the why of every addition
  is always written down.
- When the whole book plan is done, do a final README pass:
  every feature gets an explanation, its why and an example.
  Before it, clarify his goals with him, one short question at
  a time, so no hole and no false claim stays in the README.
- After every finished runfile feature, update FILE/
  README.run in the same script. Now and then refresh the AI
  intro with what the AI can do new. When the database server
  runs, the intro names its commands.
- The product uses local git always (commits, no remote). The rule No git is only for my own work on the code.
- rustup and cargo live in HOME/.cargo/bin, not on PATH in bash -c.
  Default toolchain stable 1.99.0, wasm32-unknown-unknown installed.
- Never write a done line in a todo before he confirms on screen.
  When an item is finished, delete it.
- Keep plan, todo and this file compact: when work is done, delete
  its lines at once. Never leave done work, history or long prose.
  Short lines, easy to scan.
- When you learn a better way to work on this system (a command, a path,
  a rule), write it into this file at once, as a rule. Never a log.
- Every decision and every change is written into the plan or todo
  file at once, so nothing depends on the chat.

- No apostrophe inside a bash -c single-quoted script.
- find -type f skips symlinks: use -L or the path.
- Contracts live in .runfile/TYPE/ID.rktd.
- Live runfile binary: $HOME/run-target/live/runfile/release/runfile (a symlink, find -type f misses it). Run it with 2>/dev/null, it prints a long block types line to stderr.
- After a change, end the script by opening the test page: nushell line after the bash -c block, never inside it.


- NEW PAGE: write it with a bash -c heredoc (double-quoted
  delimiter), then in the SAME block run
  /usr/local/bin/run-contracts FILE.run. It fills every contract
  and answer slot, writes them into the file and prints the open
  questions: id, block type and name, question. No editor is
  needed for contracts. --all also lists the answered ones.
- ANSWER THE CONTRACTS: follow NEW FILE FLOW below, three moves,
  three answers. Never skip the last move: he must see the raw
  file with every answer in place. Answers are short honest facts
  about that block, never filler. The id repeats for every block
  of one kind, so match by block name, never by id alone.
- FIX THE SHELL: it must run every normal TUI with the real terminal. No hard coded list of program names (is_fullscreen_line in command_runner.rs). Detect TUIs at runtime, for example by the alternate screen switch, so nothing is ever registered.

- READ THIS FILE FIRST, every session, before asking for any other
  read. Never guess a path or a command: they are written here.
- WEBSITE DRAWS STRUCTURE: the page never shows terminal text or
  color codes. The server sends blocks as data from the runfile
  parser and the page draws them as a real UI. Colors and corners
  come only from /theme.

## Server facts (verified)
- subnet-server serve: HTTP door 127.0.0.1:8080, routes /theme,
  /index.db, /cmd (POST), /ws, /file?k=KEY (file blocks as
  JSON). Log: ~/.local/state/subnet/
  server.log
- Web-interface folder: web-interface (index.html, loader.js,
  lib.rs). Last loaded runfile folder: state file runfile-root.

## How the PM works (verified, rules only)
- Manifest: manifests/NAME.rktd. Keys: source, build,
  install. Entry code: subnet/entry/subnet.rkt
  (live branch, then do-live-racket after it).
- (bin SRC DEST): SRC is read relative to the build target dir.
- Wasm manifest: add --target wasm32-unknown-unknown to the cargo argv and write bin SRC as TARGETDIR/wasm32-unknown-unknown/release/FILE.wasm.
- Wasm extern imports need #[link(wasm_import_module = "env")], else rust-lld fails with undefined symbol.
- subnet live NAME runs the manifest cargo argv in the source
  folder with --target-dir swapped for ~/run-target/live/NAME.
  Output lands in ~/run-target/live/NAME/release/.
- live: a bin SRC under the target dir is read from the live
  target. Any other bin SRC (a launcher script) is read from
  the source folder. Fixed in the pairs line of subnet.rkt.
- If any bin file is missing, live prints "binary is missing" and
  the installed version is untouched, even when cargo succeeded.
- Live links go to prefix/NAME/live. prefix = data-home/pm/prefix.
- live also links every copy-tree into live/ (fixed). runtd needs
  live/editor and the runfile package.
- With XDG_DATA_HOME empty, prefix = ~/.local/share/pm/prefix.
- A bin SRC read from the source folder must have the exec bit.
  live links it, no copy. runtd-launcher.sh needed chmod +x.
- The editor compiles runfile in by path (Cargo.toml). runtd reads
  prefix/runfile/current/bin/runfile. A runfile change needs
  subnet live editor AND subnet live runfile.
- Read manifest lines with sed -n Np | fold -w 70. Never cut them.
- Read code with sed -n A,Bp | cut -c1-110. Ask for the next range
  in the same read when the branch is long.

## Tests
- Recorded .test files are never hand-edited: delete and re-record
  with runtd record-module NAME MODULE-PATH "EXPR".
- Run one test from EDITOR/editor:
  runtd test --level 2 --only FILEBASE
  FILEBASE is the .test file name without .test. The filter matches
  the file name, not the test name. Require "^runs [1-9]" and
  "failed 0".
- Flaky under load: cursors, undo, focus-kinds, pin-view, move,
  copy-paste, block-back.
- Never run the full level 2 or level 3 run unless he asks.
- EDITOR/runtd-tutorial.run is the live test page of RUNFILE. It
  is generated by runtd docs, never hand-edited. Its RUNFILE chapters
  are read from the runfile files on every run. A new runfile
  capability is done only when it shows there.
- runtd docs must never write this file. Never add that back.
- The editor builds against FILE by path (editor/Cargo.toml).
  There is no copy of runfile inside the editor. Never make one.

## NEW FILE FLOW

Three moves, one answer each. Verified on a three paragraph page.

MOVE 1, make the page and get the contracts. One bash -c block:
write the file with a heredoc (double-quoted delimiter), then
  /usr/local/bin/run-contracts FILE.run
It fills every contract and answer slot, writes them into the
file and prints: open: N, then per question the id, block type,
block name and the question. Title needs a kind: line. Refuse if
the file exists. He pastes the output back.

MOVE 2, answer. One bash -c block, /tmp copy only:
- cp FILE to /tmp/x.run. Write /tmp/ans.awk: BEGIN holds one
  a["block-name"] = "answer text" per printed question. Rules:
  on contract start or answer start set inx to that word, on
  contract end or answer end clear it. On any other X start push
  a stack, on X end pop. On the first name: line outside a
  contract or answer, store it as the name of the stack top.
  On asked: inside an answer, print the line, then print the
  answer for that stack top name. Count hits and misses into a
  file in the END block.
- Run awk to /tmp/x2.run. Abort unless hits equal the number of
  open questions and misses are 0.
- Run run-contracts on /tmp/x2.run: it must print open: 0, else
  abort. Only then cp /tmp/x2.run over FILE, delete the tmp files.
- End the same block with: grep -v "^$" FILE | cut -c1-68

MOVE 3 is that last grep: the raw file with every contract and
answer in place. He pastes it back. Check each answer sits under
the right block, after its asked: line. Done only then.

Open the editor only when he asks to see the page: a nushell
line, runtd FILE.run, never bash -c, it needs the real tty.

- run-splash reads SPLASH_HOME or ~/.splash, never ~/.config/splash. Theme: ~/.splash/settings.toml.
- Rust inside bash -c: no char literals (apostrophe). Insert lines with a sed file: /pat/i TEXT on one line. Rename a fn BEFORE appending a new fn with the old name.
- Splash boxes in the shell show REAL splash dashboard output, never self made info.
- run-splash dump project|home WIDTH prints a dashboard as plain text, no tty needed. Heredoc with a double-quoted delimiter keeps code literal inside bash -c.

## Server (built in, no variables)
- Web-interface and runfile index are ONE server: subnet-server
  serve, HTTP 127.0.0.1:8080, socket in
  ~/.local/state/subnet/, log server.log there.
- Autostart: shell repl.rs runs "subnet serve" when the
  shell starts in a terminal. Does nothing if already running.
- Web-interface folder, in order: SUBNET_WEB_INTERFACE_DIR, state file
  web-interface-dir, state file root + "web-interface",
  then relative "web-interface". Works from any folder.
- Index: every load saves its folder to state file
  runfile-root. The server reloads it at start.
- Real index is in "FILE/.runfile". The .runfile in
  the ecosystem root is stale (11-06).
- After a build: subnet stop, then subnet serve.

## Server commands (built, verified)
- Send: curl -s --data-binary "CMD" http://127.0.0.1:8080/cmd
  or one text message on /ws. Answer is one line.
- Read: ping status load FOLDER reload links FILE backlinks NAME
  broken names git FILE. stop works only from the socket.
- CLI: runfile load reload links backlinks broken names go to the
  server over /cmd. runfile new FILE (text on stdin) creates a page
  through the server. contracts and fill write through it too. No
  server: reads exit 1, writes go straight to the file.
- Write: write PATH (content on the next lines), delete PATH,
  move FROM (target on the next line). Paths are relative to the
  loaded folder. Each is a local git commit, about 11 s a write.
- A write reindexes that file, writes the body of every live or
  copy link block into its target block file, reindexes that file
  too, then pushes the text changed to every /ws client.
- After subnet serve send load with the absolute path of RUN
  RUNFILE first, else the answer is no folder loaded and a
  write lands in the wrong folder.
- The editor save (space s) writes the file, then sends the same
  write to the server (notify_server in hot-path.rs). Without a
  server it only writes the file.

## Key w (shell input bar, Visual mode)
- w opens the web-interface (http://127.0.0.1:8080) in the phone
  browser via termux-open-url. Code: open_website in
  console-screen command_runner.rs, key arm in input_state.rs.
- Keys already taken in Visual: t h n e s c p a i x o l j k b
  g y Y / space f m M z Z d q. Free: r u v w(now used) and more.
- /theme also has fill, arrow (head, plain, none), grid (on, off).
- Demo pages with links: FILE/demo (5 files, ring).
- Block names come from the nearest .runfile/blocktypes
  folder, searched upward from the file. The root one
  (.runfile/blocktypes) had no link.blocktype, so links
  were never indexed. Copy the type there when a block
  kind is ignored. Default set: FILE/runfile/
  blocktypes (44 files, root had 43, only link was missing).
- New block types go in both blocktypes folders: the default
  FILE/runfile/blocktypes and the root .runfile/
  blocktypes. layout, card and box were added this way.
- Test layouts: FILE/demo/layout-main.run and
  layout-inner.run. Syntax: layout block with name, card
  blocks (target, place: x y), box blocks (target: layout name).
- Index kind layout-item (index-layouts.rs): fields what
  (card or box), target, place, layout (parent name), line.
- Master view: lib.rs arrange() builds frames and cards from
  layout-item records, draw() paints frames first via rect
  and text. Files in no layout are drawn by plain() below.
- Master view is HTML now: loader.js render() builds a layer
  from the wasm scene (lib.rs scene, scene_ptr: JSON with boxes,
  cards, arrow endpoints). Canvas only paints bg and grid.
- Descriptions: caption declaration in layout, card, box.
  index-layouts.rs writes it, also one record with what=layout.
- Layout width is the window width, not divided by zoom.
- ONE PLACE RULE: every runfile asset lives in FILE/
  runfile/ (blocktypes, types, layouts). Never in demo, never in
  a .runfile folder: .runfile holds only index.db and the
  root blocktypes copy, and the indexer skips it. Before creating
  or moving files, ask where the same kind already lives.
- Layout files: FILE/runfile/layouts. Indexed by the
  normal walk, no indexer change.
- Master view settings: FILE/runfile/layouts/layout-
  settings.run, block settings. Keys width height gap stepx stepy
  margin pad head curve. Indexed by index-layouts.rs (what=settings)
  and read by set_cfg in web-interface/lib.rs. Never hard code sizes.
- Master view focus: loader.js foc and applyFocus. Scene link rows
  are [x1,y1,x2,y2,from,to], from and to index into the card list.
- Temp files of a script go in their own folder under /tmp, never
  loose names like /tmp/b2: those can already be folders.
- loader.js tap: phones send a fake mouse click after touchend.
  lt holds the last touch tap time, mouseup skips within 800 ms.
- Master view scope is the scope key in runfile/layouts/layout-
  settings.run (scope: all shows every file). Read by set_cfg in
  web-interface/lib.rs. Files outside the scope stay in the index.
- Card text padding and font sizes are settings keys (padv padh
  titlesize descsize labelsize). lib.rs style(i) gives them to
  loader.js as sp(i, default).
- Settings TUI: subnet/subnet TUI configuration/settings-tui.rkt,
  run from the root with the racket line. check argument tests it.
- Scripts inside bash -c: never a bare double quote or a dollar
  sign in patch text. Write them escaped, or use a placeholder
  like DOLLAR that p.rkt turns into the sign.
- Settings pane: fifth float of the multiplexer, key e in the shell Visual
  mode. Launcher subnet/subnet TUI
  configuration/settings-pane.sh reads the root from the state
  file root. New floats: add the name in float-state-shared.rkt,
  keys in split-keys-shared.rkt and pane-split-keys.rkt, command
  and launch in mux.rkt.
- Settings TUI docs: subnet/subnet TUI
  configuration/README.md. After any change to the settings TUI
  (keys, rows, float, signal) update it in the same script.
- Settings float: key e in the shell Visual mode sends OSC 7704
  (command_runner.rs toggle_settings). The TUI sends the same
  signal to hide. It is hidden, never killed. Keys: h t n s,
  enter, e. The mux handles it in pane-grid.rkt and mux.rkt.
- Settings float prestart: mux.rkt opens float 4 after the startup fit-panes!, hidden.
- In bash -c with double quotes nushell accepts only backslash-quote and backslash-dollar. Put patterns in single quotes.
- JOB FILE: write each script as a nushell file (raw string, save) to /tmp/run/NAME.nu, then run it: ^($nu.current-exe) /tmp/run/run.nu NAME. Success deletes it, failure keeps it and prints the error. Fix with a small edit, never rewrite. Use bash only for tools that need it.

## State prefix (verified)
- Install prefix is DATA/state/prefix/NAME/current/bin (DATA=XDG_DATA_HOME or ~/.local/share), not pm/prefix.
- Verbs (entry/subnet.rkt): status, apply, dev, dev-local, live, update, path, serve, stop, devmode, edit, editor, screen, state.

## Install (verified)
- console install (entry/subnet.rkt do-install): cargo fetch, spec paths, apply, PATH, login shell. install.sh only bootstraps Racket+Rust and calls it.

## Screen (verified)
- console pane = Rust screen (console-screen lib, cli_main::run). console screen runs bin/screen (Racket mux, raco exe) next to console. Panes start as: console pane.

## History
History is console history (Atuin fork in console/history/).
Nu only. The hook and CTRL-R / Up keys are baked into the
console default config, no init step. Data dir is
~/.local/share/console. The sync server is embedded:
console history server (127.0.0.1:8888, sqlite). On shell start
console history ensure starts it and registers once.

## Garbage
Git branch garbage keeps everything deleted. Snapshot before
each deletion. Never delete the branch.
