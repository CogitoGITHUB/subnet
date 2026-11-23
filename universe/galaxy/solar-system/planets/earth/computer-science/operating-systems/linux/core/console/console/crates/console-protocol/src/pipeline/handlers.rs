use core::fmt;
use std::fmt::Display;
use std::sync::{Arc, Mutex};

use crate::{ShellError, SignalAction, engine::Sequence};

/// Handler is a closure that can be sent across threads and shared.
pub type Handler = Box<dyn Fn(SignalAction) + Send + Sync>;

/// Manages a collection of handlers.
#[derive(Clone, derive_more::Debug, Default)]
pub struct Handlers {
    /// List of handler tuples containing an ID and the handler itself.
    #[debug("{}", debug_fmt_handlers(&self.handlers))]
    handlers: Arc<Mutex<Vec<(usize, Handler)>>>,
    /// Sequence generator for unique IDs.
    next_id: Arc<Sequence>,
}

/// HandlerGuard that unregisters a handler when dropped.
#[derive(Clone, derive_more::Debug)]
pub struct HandlerGuard {
    /// Unique ID of the handler.
    id: usize,
    /// Reference to the handlers list.
    #[debug("{}", debug_fmt_handlers(&self.handlers))]
    handlers: Arc<Mutex<Vec<(usize, Handler)>>>,
}

impl Drop for HandlerGuard {
    /// Drops the `Guard`, removing the associated handler from the list.
    fn drop(&mut self) {
        if let Ok(mut handlers) = self.handlers.lock() {
            handlers.retain(|(id, _)| *id != self.id);
        }
    }
}

impl Handlers {
    pub fn new() -> Handlers {
        Self::default()
    }

    /// Registers a new handler and returns an RAII guard which will unregister the handler when
    /// dropped.
    pub fn register(&self, handler: Handler) -> Result<HandlerGuard, ShellError> {
        let id = self.next_id.next()?;
        if let Ok(mut handlers) = self.handlers.lock() {
            handlers.push((id, handler));
        }

        Ok(HandlerGuard {
            id,
            handlers: Arc::clone(&self.handlers),
        })
    }

    /// Registers a new handler which persists for the entire process lifetime.
    ///
    /// Only use this for handlers which should exist for the lifetime of the program.
    /// You should prefer to use `register` with a `HandlerGuard` when possible.
    pub fn register_unguarded(&self, handler: Handler) -> Result<(), ShellError> {
        let id = self.next_id.next()?;
        if let Ok(mut handlers) = self.handlers.lock() {
            handlers.push((id, handler));
        }

        Ok(())
    }

    /// Runs all registered handlers.
    pub fn run(&self, action: SignalAction) {
        if let Ok(handlers) = self.handlers.lock() {
            for (_, handler) in handlers.iter() {
                handler(action);
            }
        }
    }
}

#[inline]
#[expect(unused, reason = "used in `Debug` impls")]
fn debug_fmt_handlers(handlers: &Mutex<Vec<(usize, Handler)>>) -> impl Display {
    fmt::from_fn(|f| match handlers.try_lock() {
        Err(err) => write!(f, "{err:?}"),
        Ok(handlers) => {
            let ids: Vec<_> = handlers.iter().map(|(id, _)| id).collect();
            write!(f, "{ids:?}")
        }
    })
}

