#lang racket/base

(require racket/file
         racket/list
         racket/string
         "../machine/directories-shared.rkt"
         "../machine/run.rkt")

(define (state-cli)
  (define named (getenv "CONSOLE_PM_CLI"))
  (if (and named (not (string=? named "")))
      (path->complete-path named)
      (let loop ([at (simplify-path (current-directory))])
        (define up (simplify-path (build-path at "..")))
        (define found (build-path at "state" "package-manager" "src" "entry" "cli.rkt"))
        (cond
          [(file-exists? found) found]
          [(string=? (path->string at) (path->string up)) #f]
          [else (loop up)]))))

(define (state-command cli)
  (if (string-suffix? (path->string cli) ".rkt")
      (let ([racket (racket-bin)])
        (if racket
            (list (path->string racket) (path->string cli))
            (list (path->string cli))))
      (list (path->string cli))))

(define (state-verb args)
  (define cli (state-cli))
  (cond
    [(not cli)
     (hash 'ok #f 'code -1 'out ""
           'err "the editor does not know where state is: set CONSOLE_PM_CLI to the state program")]
    [else
     (define words (append (state-command cli) (list "--format" "json") args))
     (define result (run-program (string-join (map shell-quote words) " ") 60))
     (hash-set result 'ok (= (hash-ref result 'code -1) 0))]))

(define (state-apply spec confirmation)
  (if (not confirmation)
      (hash 'ok #f 'code -1 'out ""
            'err "the install was not made: it changes the disk, and the human's confirmation is needed")
      (state-verb (list "apply" spec "--confirm"))))

(define (state-verify vault lockfile name)
  (state-verb (list "verify" vault lockfile name)))

(provide state-cli
         state-command
         state-verb
         state-apply
         state-verify)
