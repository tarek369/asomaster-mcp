//! Opening the raw stream to the app: unix socket or Windows named pipe.

use std::io;

use tokio::io::{AsyncRead, AsyncWrite};

pub trait Stream: AsyncRead + AsyncWrite + Send + Unpin + 'static {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin + 'static> Stream for T {}

pub type BoxStream = Box<dyn Stream>;

/// Whether an open error means "nobody is listening" (start the app / retry).
pub fn is_not_listening(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::NotFound
            | io::ErrorKind::ConnectionRefused
            | io::ErrorKind::AddrNotAvailable
    )
}

#[cfg(unix)]
pub async fn open(endpoint: &str) -> io::Result<BoxStream> {
    let s = tokio::net::UnixStream::connect(endpoint).await?;
    Ok(Box::new(s))
}

#[cfg(windows)]
pub async fn open(endpoint: &str) -> io::Result<BoxStream> {
    use tokio::net::windows::named_pipe::ClientOptions;
    /// `ERROR_PIPE_BUSY`: every instance is taken; the server creates the next one shortly.
    const ERROR_PIPE_BUSY: i32 = 231;
    let mut tries = 0u32;
    loop {
        match ClientOptions::new().open(endpoint) {
            Ok(c) => return Ok(Box::new(c)),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && tries < 40 => {
                tries += 1;
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(e) => return Err(e),
        }
    }
}
