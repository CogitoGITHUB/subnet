#lang racket/base
(require racket/system racket/string racket/list racket/file racket/path "update-table.rkt" "update-screen.rkt" "update-evidence.rkt" "update-drive-run.rkt")

(define (parts-text)
  (define d (build-path editor-root "docs" "parts"))
  (if (directory-exists? d)
      (string-join (for/list ([p (in-list (sort (filter (lambda (q) (regexp-match? #rx"[.]md$" (path->string q))) (directory-list d #:build? #t)) string<? #:key path->string))]) (file->string p)) "\n")
      ""))

(define (howto-section)
  (define p (build-path editor-root "docs" "parts" "how-to-make-tests.md"))
  (if (file-exists? p)
      (string-append "chapter start\ntitle start\nHow to make tests\ntitle end\n\nparagraph start\n"
                     (string-join (filter (lambda (l) (and (not (regexp-match? #rx"^#" l)) (not (string=? (string-trim l) "")))) (string-split (file->string p) "\n")) " ")
                     "\nparagraph end\n\nchapter end\n")
      ""))

(define (cli-section)
  (string-append "chapter start\ntitle start\nCommands of runtd\ntitle end\n\nparagraph start\n"
                 (string-join (list "runtd FILE opens the file in the editor, and runtd alone says a file is needed."
                                    "runtd update runs the compile, the tests at a level and the docs."
                                    "runtd test --level N runs the tests in gate/tests at that level."
                                    "runtd record NAME KEYS records a test from a real run of the keys."
                                    "runtd docs runs the test of every feature and writes this tutorial."
                                    "runtd trace is a tool verb, and the key trace is written to the trace log when SUBNET_TRACE is set.")
                              "\n")
                 "\nparagraph end\n\nchapter end\n"))
(define (do-docs)
  (define fdir (and editor-root (build-path editor-root "editor" "features")))
  (cond
    [(not (and fdir (directory-exists? fdir)))
     (emit (list "RUNTD DOCS" "RED") (list "the features folder was not found"))
     1]
    [else
     (define t0 (current-inexact-milliseconds))
     (define files
       (sort (filter (lambda (p) (regexp-match? #rx"[.]feature$" (path->string p))) (directory-list fdir #:build? #t))
             string<? #:key path->string))
     (define rows
       (for/list ([p (in-list files)])
         (define ls (test-lines p))
         (define (one k) (let ([v (line-values ls k)]) (if (pair? v) (car v) "")))
         (define tpath (let ([f (find-files (lambda (q) (regexp-match? (regexp (string-append "/" (one "test") "[.]test$")) (path->string q))) (build-path editor-root "editor"))]) (if (pair? f) (car f) (build-path editor-root "gate" "tests" (string-append (one "test") ".test")))))
         (define r (if (file-exists? tpath) (run-test tpath) (list (one "test") 0.0 #f (list "test file missing"))))
         (list (one "name") (one "keys") (line-values ls "doc") (one "ai") (cadr r) (caddr r))))
     (define good (filter (lambda (r) (list-ref r 5)) rows))
     (define run
       (string-join
        (append
         (list "title start" "kind: default" "tags: docs runtd generated" "RUNTD TUTORIAL" "title end" ""
               "comment start" "Written by runtd docs from the feature files whose tests pass. Do not edit by hand." "comment end" "")
         (append*
          (for/list ([r (in-list good)])
            (append (list "chapter start" "title start" (car r) "title end" "")
                    (append* (for/list ([d (in-list (caddr r))]) (list "paragraph start" d "paragraph end" "")))
                    (list "paragraph start" (string-append "Keys: " (cadr r)) "paragraph end" "" "chapter end" "")))))
        "\n"))
     (define md
       (string-join
        (append (list "# RUNTD starting point (written by runtd docs, do not edit by hand)" ""
                      "| feature | keys | test notes |" "|---|---|---|")
                (for/list ([r (in-list good)]) (string-append "| " (car r) " | " (cadr r) " | " (cadddr r) " |")))
        "\n"))
     (display-to-file (string-append run "\n" (runfile-section) (howto-section) "\n" (cli-section) "\n") (build-path editor-root ".." "runtd-tutorial.run") #:exists (quote truncate))
     (define ok (andmap (lambda (r) (list-ref r 5)) rows))
     (for ([r (in-list rows)]) (log-step! 2 (string-append "doc:" (car r)) (list-ref r 4)))
     (define lines (for/list ([r (in-list rows)]) (step-line (string-append "doc:" (car r)) (list-ref r 4) (list-ref r 5))))
     (emit (list "RUNTD DOCS" (format "~a  ~a of ~a features written  ~a s" (if ok "GREEN" "RED") (length good) (length rows) (secs t0)))
           lines)
     (if ok 0 1)]))



(provide parts-text howto-section cli-section do-docs)

(define (field-of ls key)
  (define pre (string-append key ": "))
  (or (for/first ([l (in-list ls)] #:when (string-prefix? l pre))
        (substring l (string-length pre)))
      ""))

(define (runfile-root)
  (build-path editor-root ".." ".." "FILE" "runfile"))

(define (blocktype-chapters)
  (define d (build-path (runfile-root) "blocktypes"))
  (define ps
    (if (directory-exists? d)
        (sort (filter (lambda (q) (regexp-match? #rx"[.]blocktype$" (path->string q)))
                      (directory-list d #:build? #t))
              string<? #:key path->string)
        (list)))
  (append*
   (for/list ([p (in-list ps)])
     (define ls (file->lines p))
     (define nm (field-of ls "name"))
     (list "chapter start" "title start" (string-append "Block: " nm) "title end" ""
           "paragraph start" (string-append "The " nm " block is " (field-of ls "kind") ".") "paragraph end" ""
           "paragraph start" (string-append "Opaque: " (field-of ls "opaque")) "paragraph end" ""
           "chapter end" ""))))

(define (filetype-names)
  (define d (build-path (runfile-root) "filetypes" "types"))
  (if (directory-exists? d)
      (sort (for/list ([p (in-list (directory-list d))]
                       #:when (let ([s (path->string p)])
                                (not (or (regexp-match? #rx"^_" s) (regexp-match? #rx"README" s)))))
              (regexp-replace #rx"[.]rkt$" (path->string p) ""))
            string<?)
      (list)))

(define (runfile-section)
  (string-append
   (string-join
    (append
     (list "chapter start" "title start" "What FILE can do" "title end" ""
           "paragraph start" "Read live from the runfile files each time runtd docs runs. A new blocktype file shows here by itself." "paragraph end" "")
     (blocktype-chapters)
     (list "chapter start" "title start" "File types" "title end" ""
           "paragraph start" (string-append "Known file types: " (string-join (filetype-names) ", ") ".") "paragraph end" ""
           "chapter end" "" "chapter end"))
    "\n")
   "\n"))
