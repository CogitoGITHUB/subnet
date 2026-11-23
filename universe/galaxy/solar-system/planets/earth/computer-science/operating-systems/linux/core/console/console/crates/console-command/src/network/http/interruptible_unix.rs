//! Interruptible Unix domain socket connector for ureq HTTP client.
//!
//! This module provides a Unix socket transport implementation that can be interrupted
//! when the user presses Ctrl+C via a registered signal handler.
//!
//! See [`super::interruptible_tcp`] for a detailed explanation of the interrupt strategy.

use std::fmt;
use std::io::{Read, Write};
use std::mem::ManuallyDrop;
#[cfg(unix)]
use std::net::Shutdown;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(windows)]
use win_uds::net::UnixStream;

use console_protocol::HandlerGuard;
use ureq::Error;
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, LazyBuffers, NextTimeout, Transport,
};

/// Callback invoked when a Unix socket connection is established.
pub type OnConnectUnix =
    Arc<dyn Fn(&UnixStream) -> Option<(HandlerGuard, Arc<AtomicBool>)> + Send + Sync>;

/// Connector for interruptible Unix domain sockets.
pub struct InterruptibleUnixSocketConnector {
    socket_path: PathBuf,
    on_connect: Option<OnConnectUnix>,
}

impl InterruptibleUnixSocketConnector {
    pub fn new(socket_path: PathBuf, on_connect: Option<OnConnectUnix>) -> Self {
        Self {
            socket_path,
            on_connect,
        }
    }
}

impl fmt::Debug for InterruptibleUnixSocketConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InterruptibleUnixSocketConnector")
            .field("socket_path", &self.socket_path)
            .field("on_connect", &self.on_connect.as_ref().map(|_| "..."))
            .finish()
    }
}

impl<In: Transport> Connector<In> for InterruptibleUnixSocketConnector {
    type Out = InterruptibleUnixSocketTransport;

    fn connect(
        &self,
        details: &ConnectionDetails,
        _chained: Option<In>,
    ) -> Result<Option<Self::Out>, Error> {
        // Connect to the Unix socket, ignoring the URI's host/port
        let stream = UnixStream::connect(&self.socket_path).map_err(|e| {
            Error::Io(std::io::Error::new(
                e.kind(),
                format!(
                    "Failed to connect to Unix socket {:?}: {}",
                    self.socket_path, e
                ),
            ))
        })?;

        let (guard, closed) = self
            .on_connect
            .as_ref()
            .and_then(|f| f(&stream))
            .map(|(g, c)| (Some(g), c))
            .unwrap_or_else(|| (None, Arc::new(AtomicBool::new(false))));

        let buffers = LazyBuffers::new(
            details.config.input_buffer_size(),
            details.config.output_buffer_size(),
        );

        Ok(Some(InterruptibleUnixSocketTransport::new(
            stream, buffers, guard, closed,
        )))
    }
}

/// Transport implementation for interruptible Unix domain sockets.
pub struct InterruptibleUnixSocketTransport {
    stream: ManuallyDrop<UnixStream>,
    buffers: LazyBuffers,
    timeout_write: Option<Duration>,
    timeout_read: Option<Duration>,
    closed: Arc<AtomicBool>,
    _guard: Option<HandlerGuard>,
}

impl InterruptibleUnixSocketTransport {
    pub fn new(
        stream: UnixStream,
        buffers: LazyBuffers,
        guard: Option<HandlerGuard>,
        closed: Arc<AtomicBool>,
    ) -> Self {
        Self {
            stream: ManuallyDrop::new(stream),
            buffers,
            timeout_read: None,
            timeout_write: None,
            closed,
            _guard: guard,
        }
    }
}

impl Drop for InterruptibleUnixSocketTransport {
    fn drop(&mut self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            // SAFETY: We're in Drop, stream won't be used after this.
            unsafe { ManuallyDrop::drop(&mut self.stream) };
        }
    }
}

impl Transport for InterruptibleUnixSocketTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error> {
        let maybe_timeout = timeout.not_zero().map(|t| *t);
        if maybe_timeout != self.timeout_write {
            self.stream
                .set_write_timeout(maybe_timeout)
                .map_err(Error::Io)?;
            self.timeout_write = maybe_timeout;
        }

        let output = &self.buffers.output()[..amount];
        match self.stream.write_all(output) {
            Ok(()) => Ok(()),
            Err(e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                Err(Error::Timeout(timeout.reason))
            }
            Err(e) => Err(Error::Io(e)),
        }
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        let maybe_timeout = timeout.not_zero().map(|t| *t);
        if maybe_timeout != self.timeout_read {
            self.stream
                .set_read_timeout(maybe_timeout)
                .map_err(Error::Io)?;
            self.timeout_read = maybe_timeout;
        }

        let input = self.buffers.input_append_buf();
        let amount = match self.stream.read(input) {
            Ok(n) => n,
            Err(e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                return Err(Error::Timeout(timeout.reason));
            }
            Err(e) => return Err(Error::Io(e)),
        };
        self.buffers.input_appended(amount);
        Ok(amount > 0)
    }

    fn is_open(&mut self) -> bool {
        // Unix sockets don't have a reliable way to check connection status
        // without using unstable features, so we assume the connection is open.
        // The connection will be detected as closed when we try to read/write.
        true
    }
}

impl fmt::Debug for InterruptibleUnixSocketTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InterruptibleUnixSocketTransport")
            .field("peer_addr", &self.stream.peer_addr().ok())
            .finish()
    }
}

/// Create an `OnConnectUnix` callback that registers a signal handler to interrupt the socket.
///
/// See [`super::interruptible_tcp::make_on_connect`] for the interrupt strategy.
pub fn make_on_connect_unix(handlers: &console_protocol::Handlers) -> OnConnectUnix {
    let handlers = handlers.clone();
    Arc::new(move |stream: &UnixStream| {
        let closed = Arc::new(AtomicBool::new(false));

        #[cfg(unix)]
        let guard = {
            let clone = stream.try_clone().ok()?;
            handlers
                .register(Box::new(move |action| {
                    if matches!(action, console_protocol::SignalAction::Interrupt) {
                        let _ = clone.shutdown(Shutdown::Both);
                    }
                }))
                .ok()?
        };

        #[cfg(windows)]
        let guard = {
            use std::os::windows::io::AsRawSocket;
            super::interruptible_tcp::register_close_handler(
                &handlers,
                stream.as_raw_socket(),
                &closed,
            )?
        };

        Some((guard, closed))
    })
}

