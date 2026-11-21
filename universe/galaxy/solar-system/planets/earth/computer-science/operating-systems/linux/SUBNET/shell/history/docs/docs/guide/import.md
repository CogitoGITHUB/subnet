# Import existing history

shell-history uses a shell plugin to ensure that we capture new shell history. But for
older history, you will need to import it

This will import the history for your current shell:
```
shell-history import auto
```

Alternatively, you can specify the shell like so:

```
shell-history import bash
shell-history import zsh # etc
```

Your old shell history file will continue to be updated, regardless of shell-history usage.
