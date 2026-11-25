(spec
  (format-version 2)
  (name console)
  (version "0.1.0")
  (summary "the RUN console")
  (homepage "https://github.com/CogitoGITHUB/subnet")
  (source (local "/root/home/shape/subnet/universe/galaxy/solar-system/planets/earth/computer-science/operating-systems/linux/core/console"))
  (build (steps
   (run #:argv ("cargo" "build" "--release"
     "--target-dir" "target-console" "--manifest-path"
     "console/Cargo.toml" "--bin" "console"))
   (run #:argv ("cargo" "build" "--release"
     "--target-dir" "runfile/target" "--manifest-path"
     "runfile/Cargo.toml"))
   (run #:argv ("cargo" "build" "--release"
     "--target-dir" "brochure/target" "--manifest-path"
     "brochure/Cargo.toml"))
   (run #:argv ("cargo" "build" "--release"
     "--target-dir" "target-server" "--manifest-path"
     "server/Cargo.toml"))
   (run #:argv ("cargo" "build" "--target-dir"
     "target-hot-path" "--manifest-path"
     "editor/editor/editor/Cargo.toml" "--bin" "hot-path"))
   (run #:argv ("raco" "exe" "-o" "screen-built"
     "screen/multiplexer/entry/mux.rkt"))
   (run #:argv ("raco" "exe" "-o" "console-built"
     "entry/entry/console.rkt"))
   (run #:argv ("cargo" "build" "--release"
     "--target" "wasm32-unknown-unknown" "--target-dir"
     "web-interface/target" "--manifest-path"
     "web-interface/Cargo.toml"))))
  (install (prefix
   (copy-tree "state" "state")
   (copy-tree "editor" "editor")
   (copy-tree "runfile" "runfile")
   (copy-tree "screen" "screen")
   (bin "target-console/release/console" "bin/console")
   (bin "runfile/target/release/runfile" "bin/runfile")
   (bin "brochure/target/release/brochure" "bin/brochure")
   (bin "target-server/release/console-server"
        "bin/console-server")
   (bin "target-hot-path/debug/hot-path" "bin/hot-path")
   (bin "editor/console-install/runtd-launcher.sh"
        "bin/runtd")
   (bin "screen-built" "bin/screen")
   (bin "console-built" "bin/console-entry")
   (bin "web-interface/target/wasm32-unknown-unknown/release/bridge.wasm" "bin/bridge.wasm")))
  (license mit))
