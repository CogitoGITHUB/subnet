#lang racket/base
(require "pane-pty.rkt" "pane-grid.rkt")
(void (let* ([home (or (getenv "HOME") "")]
       [screen (string-append home "/.local/share/pm/prefix/console/current/bin/console")]
       [console-bin (string-append home "/.local/share/pm/prefix/console/current/bin/console")])
  (when (and (not (getenv "CONSOLE_PANE_PROGRAM")) (file-exists? screen))
    (putenv "CONSOLE_PANE_PROGRAM" screen))
  (when (and (not (getenv "CONSOLE_BINARY")) (file-exists? console-bin))
    (putenv "CONSOLE_BINARY" console-bin))
  (when (getenv "CONSOLE_PANE_PROGRAM")
    (putenv "CONSOLE_IN_MULTIPLEXER" "1"))))
(provide (struct-out pane-unit) pane-unit-open pane-unit-open-program pane-unit-close!)

(struct pane-unit (id grid link scroll))

(define (pane-unit-open id columns rows directory)
  (define grid (make-pane-grid columns rows))
  (define console-bin (or (getenv "CONSOLE_PANE_PROGRAM") (path->string (or (find-executable-path "nu") (error "nu not found")))))
  (define link
    (parameterize ([current-directory (if (and (string? directory) (> (string-length directory) 0) (directory-exists? directory)) directory (current-directory))])
      (pane-open console-bin (if (getenv "CONSOLE_PANE_PROGRAM") (list "pane") (list)) columns rows)))
  (set-pane-grid-reply!
   grid
   (lambda (answer)
     (write-bytes answer (pane-link-master-out link))
     (flush-output (pane-link-master-out link))))
  (pane-unit id grid link (box 0)))

(define (pane-unit-close! unit)
  (pane-close! (pane-unit-link unit)))

(define (pane-unit-open-program id columns rows program arguments)
  (define grid (make-pane-grid columns rows))
  (define link (pane-open program arguments columns rows))
  (set-pane-grid-reply!
   grid
   (lambda (answer)
     (write-bytes answer (pane-link-master-out link))
     (flush-output (pane-link-master-out link))))
  (pane-unit id grid link (box 0)))
