use console_protocol::engine::{EngineState, Jobs};
use console_protocol::{ShellWarning, report_shell_warning};
use std::sync::{Mutex, PoisonError, atomic::Ordering};

/// Exit the process or clean jobs if appropriate.
///
/// In a REPL session, warns if any jobs are not `auto_kill` and
/// `exit_warning_given` is false. Sets that flag and returns `tag` in that case.
/// Otherwise, kills all jobs, drops `tag`, and exits the current process.
///
// Currently, this `tag` argument exists mostly so that a LineEditor can be dropped before exiting the process.
pub fn cleanup_exit<T>(tag: T, engine_state: &EngineState, exit_code: i32) -> T {
    if let Some(tag) = cleanup(tag, engine_state) {
        return tag;
    }

    std::process::exit(exit_code);
}

/// Clean up jobs before exiting, unless the REPL needs to warn about running jobs.
///
/// In a REPL session, warns if any jobs are not `auto_kill` and
/// `exit_warning_given` is false. Sets that flag and returns `Some(tag)` in that case.
/// Otherwise, kills all jobs, drops `tag`, and returns `None`.
pub fn cleanup<T>(tag: T, engine_state: &EngineState) -> Option<T> {
    let jobs = engine_state
        .jobs
        .lock()
        .unwrap_or_else(PoisonError::into_inner);

    let job_count = jobs.iter().filter(|(_, job)| !job.is_auto_kill()).count();

    if engine_state.is_repl
        && job_count > 0
        && !engine_state.exit_warning_given.load(Ordering::SeqCst)
    {
        report_shell_warning(
            None,
            engine_state,
            &ShellWarning::BackgroundJobsRunning { job_count },
        );

        engine_state
            .exit_warning_given
            .store(true, Ordering::SeqCst);

        return Some(tag);
    }

    drop(jobs);
    kill_all_jobs(&engine_state.jobs);

    drop(tag);
    None
}

/// Kill all background jobs, then exit the current process.
///
/// Unlike [`cleanup_exit`], which in a REPL session warns about running jobs and keeps Nu
/// running instead, this always kills them and exits. Use it where Nu exits without returning to
/// a prompt, such as when commands (`-c`) or a script finish or fail, even with `-i`: jobs still
/// in the table would outlive Nu.
pub fn kill_jobs_and_exit(engine_state: &EngineState, exit_code: i32) -> ! {
    kill_all_jobs(&engine_state.jobs);
    std::process::exit(exit_code)
}

/// Kill all background jobs and remove them from the job table, without warning.
///
/// For when Nu exits without a prompt to warn at; see [`kill_jobs_and_exit`]. A poisoned job
/// table is still cleaned up, since its jobs would otherwise outlive Nu.
pub fn kill_all_jobs(jobs: &Mutex<Jobs>) {
    let mut jobs = jobs.lock().unwrap_or_else(PoisonError::into_inner);
    let _ = jobs.kill_all();
}

