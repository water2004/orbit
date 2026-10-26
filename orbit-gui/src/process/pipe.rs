//! A command owns its output only until its process exits. Descendants may
//! retain inherited writer handles; waiting for their EOF would keep a finite
//! CLI task alive for the entire lifetime of a detached server.
use std::io::{self, Read};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

pub(super) struct CommandPipe<R> {
    pipe: R,
    process_finished: Arc<AtomicBool>,
    remaining: Option<usize>,
}

pub(super) trait PollPipe: Read {
    fn prepare(&self) -> io::Result<()>;
    fn read_available(&mut self, bytes: &mut [u8]) -> io::Result<usize>;
    fn buffered_bytes(&self) -> io::Result<usize>;
}

impl<R: PollPipe> CommandPipe<R> {
    pub(super) fn new(pipe: R, process_finished: Arc<AtomicBool>) -> io::Result<Self> {
        pipe.prepare()?;
        Ok(Self {
            pipe,
            process_finished,
            remaining: None,
        })
    }
}

impl<R: PollPipe> Read for CommandPipe<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            // Freeze the unread byte count once the direct CLI has exited.
            // A still-chatty descendant must not keep extending this task.
            if self.remaining.is_none() && self.process_finished.load(Ordering::Acquire) {
                self.remaining = Some(self.pipe.buffered_bytes()?);
            }
            let count = self.remaining.unwrap_or(bytes.len()).min(bytes.len());
            if count == 0 {
                return Ok(0);
            }
            match self.pipe.read_available(&mut bytes[..count]) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if self.remaining.is_some() {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "CLI pipe lost buffered output",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Ok(read) => {
                    if let Some(remaining) = &mut self.remaining {
                        *remaining -= read;
                    }
                    return Ok(read);
                }
                result => return result,
            }
        }
    }
}

#[cfg(windows)]
impl<R: Read + std::os::windows::io::AsRawHandle> PollPipe for R {
    fn prepare(&self) -> io::Result<()> {
        Ok(())
    }
    fn read_available(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let available = self.buffered_bytes()?;
        if available == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let count = bytes.len().min(available);
        self.read(&mut bytes[..count])
    }
    fn buffered_bytes(&self) -> io::Result<usize> {
        let mut available = 0;
        // SAFETY: the owned pipe handle stays alive, available is writable, and
        // this reader is its sole consumer. No competing synchronous read can
        // hold the handle between PeekNamedPipe and Read.
        let result = unsafe {
            windows_sys::Win32::System::Pipes::PeekNamedPipe(
                self.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        };
        if result == 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::BrokenPipe {
                Ok(0)
            } else {
                Err(error)
            };
        }
        Ok(available as usize)
    }
}

#[cfg(unix)]
impl<R: Read + std::os::fd::AsRawFd> PollPipe for R {
    fn prepare(&self) -> io::Result<()> {
        // SAFETY: fcntl only changes the owned read descriptor's flags. The
        // write ends belong to different open file descriptions.
        let flags = unsafe { libc::fcntl(self.as_raw_fd(), libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::fcntl(self.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    fn read_available(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.read(bytes)
    }
    fn buffered_bytes(&self) -> io::Result<usize> {
        let mut available: libc::c_int = 0;
        // SAFETY: FIONREAD writes one c_int; this owns a live pipe read end.
        if unsafe { libc::ioctl(self.as_raw_fd(), libc::FIONREAD, &mut available) } < 0 {
            return Err(io::Error::last_os_error());
        }
        usize::try_from(available)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid CLI pipe byte count"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct ChattyPipe;
    impl Read for ChattyPipe {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            bytes.fill(b'x');
            Ok(bytes.len())
        }
    }
    impl PollPipe for ChattyPipe {
        fn prepare(&self) -> io::Result<()> {
            Ok(())
        }
        fn read_available(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.read(bytes)
        }
        fn buffered_bytes(&self) -> io::Result<usize> {
            Ok(42)
        }
    }
    #[test]
    fn descendants_cannot_extend_the_frozen_drain_budget() {
        let pipe = CommandPipe::new(ChattyPipe, Arc::new(AtomicBool::new(true))).unwrap();
        let mut bytes = Vec::new();
        pipe.take(100).read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes.len(), 42);
    }
}
