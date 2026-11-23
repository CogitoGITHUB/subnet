# splash — render on new shell and on directory change
$ExecutionContext.InvokeCommand.LocationChangedAction = {
    & splash --on-cd
}
& splash
