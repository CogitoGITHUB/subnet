use console_engine::exit::kill_all_jobs;
use console_protocol::{
    Handlers, SignalAction, Signals,
    engine::{EngineState, Jobs},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// Install the Ctrl-C handler, along with the engine's [`Signals`] and signal [`Handlers`] it
/// drives.
///
/// Outside the REPL, Ctrl-C kills background jobs before publishing the interrupt: commands and
/// scripts, even with `-i`, never return to a prompt where the jobs could be managed. The REPL
/// keeps its background and frozen jobs when the prompt or a foreground command is interrupted.
pub(crate) fn ctrlc_protection(engine_state: &mut EngineState, is_repl: bool) {
    let interrupt = Arc::new(AtomicBool::new(false));
    engine_state.set_signals(Signals::new(interrupt.clone()));

    let signal_handlers = Handlers::new();
    engine_state.signal_handlers = Some(signal_handlers.clone());
    let jobs = engine_state.jobs.clone();

    ctrlc::set_handler(move || {
        handle_interrupt(&interrupt, &signal_handlers, &jobs, is_repl);
    })
    .expect("Error setting Ctrl-C handler");
}

fn handle_interrupt(
    interrupt: &AtomicBool,
    signal_handlers: &Handlers,
    jobs: &Mutex<Jobs>,
    is_repl: bool,
) {
    if !is_repl {
        // Finish cleanup before evaluation can catch the interrupt and spawn new work.
        kill_all_jobs(jobs);
    }
    interrupt.store(true, Ordering::Relaxed);
    signal_handlers.run(SignalAction::Interrupt);
}

