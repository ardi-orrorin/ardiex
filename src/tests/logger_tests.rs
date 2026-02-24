use super::*;

#[derive(Default)]
struct MockWriter {
    buf: Vec<u8>,
    writes: usize,
    flushes: usize,
    fail_write: bool,
    fail_flush: bool,
}

impl Write for MockWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.fail_write {
            return Err(io::Error::other("mock write failed"));
        }
        self.buf.extend_from_slice(buf);
        self.writes += 1;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.fail_flush {
            return Err(io::Error::other("mock flush failed"));
        }
        self.flushes += 1;
        Ok(())
    }
}

#[test]
fn tee_writer_writes_same_payload_to_both_targets() {
    let mut writer = TeeLogWriter::new(MockWriter::default(), MockWriter::default());
    writer.write_all(b"log line").expect("write must succeed");

    let file = writer
        .file_writer
        .lock()
        .expect("file writer mutex must not be poisoned");
    let stdout = writer
        .stdout
        .lock()
        .expect("stdout writer mutex must not be poisoned");

    assert_eq!(file.buf, b"log line");
    assert_eq!(stdout.buf, b"log line");
    assert_eq!(file.writes, 1);
    assert_eq!(stdout.writes, 1);
}

#[test]
fn tee_writer_stops_when_file_write_fails() {
    let file = MockWriter {
        fail_write: true,
        ..Default::default()
    };
    let mut writer = TeeLogWriter::new(file, MockWriter::default());

    let err = writer
        .write_all(b"payload")
        .expect_err("file write error must propagate");
    assert!(err.to_string().contains("mock write failed"));

    let stdout = writer
        .stdout
        .lock()
        .expect("stdout writer mutex must not be poisoned");
    assert_eq!(stdout.writes, 0);
    assert!(stdout.buf.is_empty());
}

#[test]
fn tee_writer_returns_stdout_error_after_file_write() {
    let stdout = MockWriter {
        fail_write: true,
        ..Default::default()
    };
    let mut writer = TeeLogWriter::new(MockWriter::default(), stdout);

    let err = writer
        .write_all(b"payload")
        .expect_err("stdout write error must propagate");
    assert!(err.to_string().contains("mock write failed"));

    let file = writer
        .file_writer
        .lock()
        .expect("file writer mutex must not be poisoned");
    assert_eq!(file.buf, b"payload");
    assert_eq!(file.writes, 1);
}

#[test]
fn tee_writer_flushes_both_targets() {
    let mut writer = TeeLogWriter::new(MockWriter::default(), MockWriter::default());
    writer.flush().expect("flush must succeed");

    let file = writer
        .file_writer
        .lock()
        .expect("file writer mutex must not be poisoned");
    let stdout = writer
        .stdout
        .lock()
        .expect("stdout writer mutex must not be poisoned");

    assert_eq!(file.flushes, 1);
    assert_eq!(stdout.flushes, 1);
}

#[test]
fn tee_writer_stops_when_file_flush_fails() {
    let file = MockWriter {
        fail_flush: true,
        ..Default::default()
    };
    let mut writer = TeeLogWriter::new(file, MockWriter::default());

    let err = writer.flush().expect_err("file flush error must propagate");
    assert!(err.to_string().contains("mock flush failed"));

    let stdout = writer
        .stdout
        .lock()
        .expect("stdout writer mutex must not be poisoned");
    assert_eq!(stdout.flushes, 0);
}

#[test]
fn tee_writer_returns_stdout_flush_error_after_file_flush() {
    let stdout = MockWriter {
        fail_flush: true,
        ..Default::default()
    };
    let mut writer = TeeLogWriter::new(MockWriter::default(), stdout);

    let err = writer
        .flush()
        .expect_err("stdout flush error must propagate");
    assert!(err.to_string().contains("mock flush failed"));

    let file = writer
        .file_writer
        .lock()
        .expect("file writer mutex must not be poisoned");
    assert_eq!(file.flushes, 1);
}
