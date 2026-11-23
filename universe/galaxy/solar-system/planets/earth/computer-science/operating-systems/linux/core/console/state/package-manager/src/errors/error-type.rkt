#lang racket/base
;; errors.rkt -- the only module that defines exception types and formats messages.

(require racket/contract/base
         racket/string
         racket/struct)

(provide
 (struct-out exn:fail:state)
 (contract-out
  ;; The set of error kinds (E-3).
  [state-kind/c contract?]
  ;; kind who message -> never returns (E-4, E-1, E-2). #:rule is the
  ;; security-rule a refusal enforces; without it nothing is added, so a
  ;; rule-less refusal renders byte-identically to one raised before.
  [raise-state-error (->* (state-kind/c symbol? string?)
                       (#:fields (listof (cons/c string? any/c))
                        #:hint (or/c string? #f)
                        #:cause (or/c exn? #f)
                        #:rule any/c)
                       any)]
  ;; exn:fail:state -> exit code per docs/ERRORS.org (D-045)
  [state-error-exit-code (-> exn:fail:state? exact-nonnegative-integer?)]
  ;; exn:fail:syntax kind -> exn:fail:state, for the spec-read boundary (D-4)
  [syntax-error->state-error (->* (exn:fail:syntax? state-kind/c)
                               (#:who symbol?)
                               exn:fail:state?)]
  ;; One predicate per kind (E-3)
  [spec-error? (-> any/c boolean?)]
  [resolve-error? (-> any/c boolean?)]
  [fetch-error? (-> any/c boolean?)]
  [verify-error? (-> any/c boolean?)]
  [build-error? (-> any/c boolean?)]
  [install-error? (-> any/c boolean?)]
  [config-error? (-> any/c boolean?)]
  [cancelled-error? (-> any/c boolean?)]
  [activate-error? (-> any/c boolean?)]
 [internal-error? (-> any/c boolean?)]))

;; ---------------------------------------------------------------------------
;; Exception type and kind contract

;; contract : the set of error kinds (E-3)
(define state-kind/c
  (or/c 'spec 'resolve 'fetch 'verify 'build 'install 'activate 'config 'cancelled 'internal))

(struct exn:fail:state exn:fail (kind fields hint cause))
;; kind   : state-kind/c                      which stage failed
;; fields : (Listof (Pairof String Any))   shown as "  field: value" lines
;; hint   : (U String #f)                  the NEXT line
;; cause  : (U Exn #f)                     wrapped lower-level exception (E-6)

;; ---------------------------------------------------------------------------
;; Raising and rendering (E-1, E-2, E-4)

;; symbol string fields hint cause -> string
;; Racket puts ';' after the message ONLY when continuation lines follow.
(define (render-message who message fields hint cause)
  (define tail
    (string-append
     (apply string-append
            (for/list ([f (in-list fields)])
              (format "\n  ~a: ~a" (car f) (cdr f))))
     (if cause (format "\n  cause: ~a" (exn-message cause)) "")
     (if hint (format "\n  hint: ~a" hint) "")))
  (if (string=? tail "")
      (format "~a: ~a" who message)
      (format "~a: ~a;~a" who message tail)))

;; security-rule -> its id string, or the value itself when it is already
;; one. WHY not security-rule-id: security.rkt requires THIS module, so
;; requiring it back would close a cycle, and the policy module stays the
;; only one that names the struct. struct->vector's element 0 is the
;; struct TYPE name ("struct:security-rule"), so the id -- the first
;; FIELD -- is element 1; reading element 0 is what made every refusal
;; print "rule: struct:security-rule" instead of the rule's name.
(define (rule-id-of r)
  (define fields (and (struct? r) (struct->vector r)))
  (if (and fields (> (vector-length fields) 1))
      (vector-ref fields 1)
      r))

;; state-kind symbol string -> never returns (E-4)
;; #:rule is the named rule this refusal enforces (S2-03, D-037). It goes
;; into the FIELDS, not the hint, because render-message already writes
;; fields out as "  key: value" lines and print-console.rkt prints that
;; message whole -- so the name reaches the terminal with no new printing
;; code anywhere, and a refusal that cites no rule prints exactly as it
;; always did (the field is not added at all).
(define (raise-state-error kind who message
                        #:fields [fields '()] #:hint [hint #f] #:cause [cause #f]
                        #:rule [rule #f])
  (define all-fields
    (if rule
        (append fields (list (cons "rule" (rule-id-of rule))))
        fields))
  (raise (exn:fail:state (render-message who message all-fields hint cause)
                       (current-continuation-marks)
                       kind all-fields hint cause)))

;; exn:fail:state -> exit code per docs/ERRORS.org (D-045). Sysexits gave
;; the two kinds that had invented numbers their outside names:
;; 'config is 78 (EX_CONFIG), 'internal is 70 (EX_SOFTWARE).
(define (state-error-exit-code e)
  (case (exn:fail:state-kind e)
    [(spec) 3] [(resolve) 4] [(fetch) 5] [(verify) 6]
    [(build) 7] [(install) 8] [(activate) 9] [(config) 78] [(cancelled) 130] [(internal) 70]
    [else 70]))

;; any -> boolean, one predicate per kind (E-3)
(define (spec-error? x) (and (exn:fail:state? x) (eq? (exn:fail:state-kind x) 'spec)))
(define (resolve-error? x) (and (exn:fail:state? x) (eq? (exn:fail:state-kind x) 'resolve)))
(define (fetch-error? x) (and (exn:fail:state? x) (eq? (exn:fail:state-kind x) 'fetch)))
(define (verify-error? x) (and (exn:fail:state? x) (eq? (exn:fail:state-kind x) 'verify)))
(define (build-error? x) (and (exn:fail:state? x) (eq? (exn:fail:state-kind x) 'build)))
(define (install-error? x) (and (exn:fail:state? x) (eq? (exn:fail:state-kind x) 'install)))
(define (activate-error? x) (and (exn:fail:state? x) (eq? (exn:fail:state-kind x) 'activate)))
(define (config-error? x) (and (exn:fail:state? x) (eq? (exn:fail:state-kind x) 'config)))
(define (cancelled-error? x) (and (exn:fail:state? x) (eq? (exn:fail:state-kind x) 'cancelled)))
(define (internal-error? x) (and (exn:fail:state? x) (eq? (exn:fail:state-kind x) 'internal)))

;; ---------------------------------------------------------------------------
;; Wrapping syntax-parse failures (D-4, E-6)

;; exn:fail:syntax state-kind -> exn:fail:state
;; Wrap a declaration failure for the spec-read boundary: srcloc in fields,
;; original exn in cause, so ui only handles one type.
(define (syntax-error->state-error e kind #:who [who 'read-spec])
  ;; Racket columns are 0-based; humans get +1 (lines already 1-based).
  (define (location-string x)
    (format "~a:~a:~a"
            (or (syntax-source x) "?")
            (or (syntax-line x) "?")
            (let ((c (syntax-column x))) (if c (+ c 1) "?"))))
  (define locs
    (for/list ([x (in-list (exn:fail:syntax-exprs e))])
      (location-string x)))
  (define fields `(("at" . ,(string-join locs ", "))))
  (exn:fail:state (render-message who "invalid declaration" fields #f e)
               (current-continuation-marks)
               kind fields #f e))

;; ---------------------------------------------------------------------------
