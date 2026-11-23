# splash — render on new shell and on directory change
if [[ -o interactive ]]; then
  __splash_on_cd() {
    command splash --on-cd
  }
  autoload -U add-zsh-hook
  add-zsh-hook chpwd __splash_on_cd
  command splash
fi
