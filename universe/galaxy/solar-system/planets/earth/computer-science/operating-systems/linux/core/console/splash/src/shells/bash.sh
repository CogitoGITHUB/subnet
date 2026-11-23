# splash — render on new shell and on directory change
if [[ $- == *i* ]]; then
  __splash_on_chpwd() {
    if [[ "$PWD" != "$__SPLASH_LAST_PWD" ]]; then
      __SPLASH_LAST_PWD="$PWD"
      command splash --on-cd
    fi
  }
  __SPLASH_LAST_PWD="$PWD"
  PROMPT_COMMAND="__splash_on_chpwd${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
  command splash
fi
