//! A disappearing observer must not unwind the game runner. Other I/O errors
//! remain errors; the bounded Launcher-owned logs do not depend on these pipes.
use std::{
    fmt,
    io::{self, Write},
    sync::atomic::{AtomicBool, Ordering},
};
static STDOUT_CLOSED: AtomicBool = AtomicBool::new(false);
static STDERR_CLOSED: AtomicBool = AtomicBool::new(false);
fn emit(writer: &mut impl Write, args: fmt::Arguments<'_>) -> io::Result<bool> {
    match writer
        .write_fmt(args)
        .and_then(|()| writer.write_all(b"\n"))
        .and_then(|()| writer.flush())
    {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(false),
        Err(error) => Err(error),
    }
}
pub fn write_line(stderr: bool, args: fmt::Arguments<'_>) -> io::Result<()> {
    let closed = if stderr {
        &STDERR_CLOSED
    } else {
        &STDOUT_CLOSED
    };
    if closed.load(Ordering::Relaxed) {
        return Ok(());
    }
    let connected = if stderr {
        emit(&mut io::stderr().lock(), args)?
    } else {
        emit(&mut io::stdout().lock(), args)?
    };
    if !connected {
        closed.store(true, Ordering::Relaxed);
    }
    Ok(())
}
macro_rules! outln {
    () => { outln!("") };
    ($($argument:tt)*) => {
        crate::output_sink::write_line(false, format_args!($($argument)*))
            .expect("cannot write CLI stdout")
    };
}
macro_rules! errln {
    () => { errln!("") };
    ($($argument:tt)*) => {
        crate::output_sink::write_line(true, format_args!($($argument)*))
            .expect("cannot write CLI stderr")
    };
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Failed(io::ErrorKind);
    impl Write for Failed {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(self.0))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn only_closed_observers_are_ignored() {
        assert!(
            !emit(
                &mut Failed(io::ErrorKind::BrokenPipe),
                format_args!("game output"),
            )
            .unwrap()
        );
        assert!(
            emit(
                &mut Failed(io::ErrorKind::PermissionDenied),
                format_args!("game output"),
            )
            .is_err()
        );
    }
}
