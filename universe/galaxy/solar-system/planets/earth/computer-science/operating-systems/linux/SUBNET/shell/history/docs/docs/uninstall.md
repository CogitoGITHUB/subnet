# Uninstalling shell-history

Sorry to see you go!

If you used the shell-history installer, you can totally delete it by removing the following

1. Delete the `~/.shell-history` directory
2. Delete the `~/.config/shell-history` directory
3. Delete the `~/.local/share/shell-history` directory
4. Remove the line referencing "shell-history init" from your shell config
5. Fish users: delete `~/.config/fish/conf.d/shell-history.env.fish` if it exists

Otherwise, uninstalling shell-history depends on your system, and how you installed it.

EG, on macOS, you'd want to run

```
brew uninstall shell-history
```

and then remove the shell integration.
