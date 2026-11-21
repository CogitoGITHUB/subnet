#lang racket/base

(define runfile-tail (list "FILE" "runfile" "target" "debug" "runfile"))

(define (runfile-bin-search dir)
  (define named (getenv "RUNFILE_BIN"))
  (if (and named (not (string=? named "")) (file-exists? named))
      named
      (let loop ([d (path->complete-path dir)] [k 0])
        (define c (apply build-path d runfile-tail))
        (cond [(file-exists? c) (path->string c)]
              [(> k 8) #f]
              [else (let-values ([(base name dir?) (split-path d)])
                      (if (path? base) (loop base (add1 k)) #f))]))))

(provide runfile-tail runfile-bin-from)
(define (runfile-bin-from dir)
  (define home (getenv "HOME"))
  (define xdg (getenv "XDG_DATA_HOME"))
  (define data (if (and xdg (not (string=? xdg ""))) xdg (and home (string-append home "/.local/share"))))
  (define pm (and data (string-append data "/pm/prefix/runfile/current/bin/runfile")))
  (or (runfile-bin-search dir) (and pm (file-exists? pm) pm)))
