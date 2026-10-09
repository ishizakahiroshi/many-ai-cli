use std::io::{self, Write};

pub(crate) fn write_diagnostic(message: &str) {
    write_diagnostic_to(&mut io::stderr().lock(), message);
}

fn write_diagnostic_to(stderr: &mut impl Write, message: &str) {
    // A background Hub can outlive the terminal that supplied stderr. A
    // diagnostic sink failure must not unwind the caller's completed work.
    let _ = stderr.write_all(message.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ClosedStderr {
        attempts: usize,
    }
    impl Write for ClosedStderr {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            self.attempts += 1;
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "synthetic closed stderr",
            ))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn application_diagnostics_survive_closed_stderr_and_preserve_healthy_output() {
        let messages = [
            "Hub diagnostic log close unavailable\n",
            "Hub preference media: synthetic failure\n",
            "sessionstore: search index update failed; message saved\n",
        ];
        let mut closed = ClosedStderr { attempts: 0 };
        let mut healthy = Vec::new();
        for message in messages {
            write_diagnostic_to(&mut closed, message);
            write_diagnostic_to(&mut healthy, message);
        }
        assert_eq!(closed.attempts, messages.len());
        assert_eq!(healthy, messages.concat().as_bytes());
    }
}
