# splash — render on new shell and on directory change
if status is-interactive
    function __splash_on_cd --on-variable PWD
        command splash --on-cd
    end
    command splash
end
