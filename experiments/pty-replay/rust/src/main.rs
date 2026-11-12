#![forbid(unsafe_code)]

mod replay;

use replay::{Replay, LIMIT};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::hint::black_box;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

const PACED_OPERATIONS: usize = 1500;
const PACED_INTERVAL: Duration = Duration::from_millis(20);
const WARMUP_DURATION: Duration = Duration::from_secs(3);

type Result<T> = std::result::Result<T, String>;

struct Options {
    mode: String,
    fixture_dir: PathBuf,
    repetitions: u64,
}

struct Fixture {
    payload: Vec<u8>,
    boundaries: Vec<(usize, usize)>,
}

impl Fixture {
    fn chunk(&self, index: usize) -> &[u8] {
        let (start, end) = self.boundaries[index];
        &self.payload[start..end]
    }
}

struct Measurements {
    wall_ms: f64,
    snapshot_ms: f64,
    expected_bytes: u64,
    completed_bytes: u64,
    expected_ops: u64,
    completed_ops: u64,
    total: i64,
    max_lateness_ms: f64,
}

impl Measurements {
    fn scalar_fields(&self, mode: &str) -> String {
        // mode is validated against fixed ASCII choices before reaching here.
        format!(
            "\"language\":\"rust\",\"mode\":\"{}\",\"wall_ms\":{:.9},\"snapshot_ms\":{:.9},\"expected_bytes\":{},\"completed_bytes\":{},\"expected_ops\":{},\"completed_ops\":{},\"total\":{},\"max_lateness_ms\":{:.9}",
            mode, self.wall_ms, self.snapshot_ms, self.expected_bytes,
            self.completed_bytes, self.expected_ops, self.completed_ops,
            self.total, self.max_lateness_ms
        )
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("replay-bench-rust: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    if usize::BITS != 64 {
        return Err("a 64-bit executable is required".into());
    }
    let options = parse_options(env::args().skip(1).collect())?;
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    if options.mode == "verify" {
        verify(&options.fixture_dir, &mut out)
    } else {
        let stdin = io::stdin();
        let mut input = BufReader::new(stdin.lock());
        measure(&options, &mut input, &mut out)
    }
}

fn parse_options(args: Vec<String>) -> Result<Options> {
    let mut mode = String::new();
    let mut fixture_dir = PathBuf::new();
    let mut repetitions = 1;
    let mut seen = HashSet::new();
    let mut index = 0;
    while index < args.len() {
        let key = &args[index];
        if index + 1 == args.len() || !seen.insert(key.clone()) {
            return Err(format!("missing value or duplicate option: {key}"));
        }
        let value = &args[index + 1];
        match key.as_str() {
            "--mode" => mode = value.clone(),
            "--fixture-dir" => fixture_dir = PathBuf::from(value),
            "--repetitions" => {
                repetitions = decimal(value)?;
                if repetitions == 0 {
                    return Err("repetitions must be a positive decimal int64".into());
                }
            }
            _ => return Err(format!("unknown option: {key}")),
        }
        index += 2;
    }
    if !matches!(mode.as_str(), "verify" | "paced" | "saturated") {
        return Err("--mode must be verify, paced, or saturated".into());
    }
    if fixture_dir.as_os_str().is_empty() {
        return Err("--fixture-dir is required".into());
    }
    Ok(Options { mode, fixture_dir, repetitions })
}

fn decimal(text: &str) -> Result<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("expected unsigned decimal int64".into());
    }
    let value = text.parse::<u64>().map_err(|_| "decimal overflow".to_string())?;
    if value > i64::MAX as u64 {
        return Err("decimal exceeds int64".into());
    }
    Ok(value)
}

fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.ends_with('.')
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
        })
}

fn read_lines(path: &Path) -> Result<Vec<String>> {
    let contents = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let normalized = contents.replace("\r\n", "\n");
    let text = normalized.strip_suffix('\n').unwrap_or(&normalized);
    if text.is_empty() {
        return Err("empty records file".into());
    }
    text.split('\n')
        .map(|line| {
            if line.is_empty() || line.contains('\r') {
                Err("blank or malformed record".into())
            } else {
                Ok(line.to_string())
            }
        })
        .collect()
}

fn load_fixture(dir: &Path, payload_name: &str, chunks_name: &str, allow_zero: bool) -> Result<Fixture> {
    if !safe_name(payload_name) || !safe_name(chunks_name) {
        return Err("fixture paths must be simple ASCII filenames".into());
    }
    let payload = fs::read(dir.join(payload_name)).map_err(|error| error.to_string())?;
    let lines = read_lines(&dir.join(chunks_name))?;
    let mut boundaries = Vec::with_capacity(lines.len());
    let mut position = 0;
    for line in lines {
        let length = decimal(&line)?;
        if (!allow_zero && length == 0) || length > (payload.len() - position) as u64 {
            return Err(format!("invalid boundary in {chunks_name}"));
        }
        let end = position + length as usize;
        boundaries.push((position, end));
        position = end;
    }
    if position != payload.len() {
        return Err(format!("boundary sum differs from payload length: {payload_name}"));
    }
    Ok(Fixture { payload, boundaries })
}

// The oracle is the suffix of the original concatenated input prefix; it does
// not call Replay or reproduce Replay's compaction/growth algorithm.
fn check_snapshot(replay: &Replay, prefix: &[u8]) -> Result<()> {
    let expected = &prefix[prefix.len().saturating_sub(LIMIT)..];
    let (mut got, total) = replay.snapshot();
    if total != prefix.len() as i64 || got.as_slice() != expected {
        return Err("snapshot bytes or total differ from concat-and-tail oracle".into());
    }
    for byte in &mut got {
        *byte ^= 0xff;
    }
    let (again, total) = replay.snapshot();
    if total != prefix.len() as i64 || again.as_slice() != expected {
        return Err("snapshot aliases retained bytes".into());
    }
    Ok(())
}

fn verify(dir: &Path, out: &mut impl Write) -> Result<()> {
    let lines = read_lines(&dir.join("cases.tsv"))?;
    let mut names = HashSet::new();
    let mut checkpoints = 0;
    for line in &lines {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 3 || !safe_name(fields[0]) || !names.insert(fields[0]) {
            return Err("invalid or duplicate cases.tsv record".into());
        }
        let fixture = load_fixture(dir, fields[1], fields[2], true)
            .map_err(|error| format!("case {}: {error}", fields[0]))?;
        let replay = Replay::default();
        check_snapshot(&replay, &[])
            .map_err(|error| format!("case {} initial: {error}", fields[0]))?;
        checkpoints += 1;
        let mut position = 0;
        for index in 0..fixture.boundaries.len() {
            let mut input = fixture.chunk(index).to_vec();
            replay.append(&input);
            for byte in &mut input {
                *byte ^= 0xff;
            }
            position += input.len();
            if index < 16 || (index + 1) % 64 == 0 || index + 1 == fixture.boundaries.len() {
                check_snapshot(&replay, &fixture.payload[..position])
                    .map_err(|error| format!("case {} chunk {}: {error}", fields[0], index + 1))?;
                checkpoints += 1;
            }
        }
    }
    verify_concurrent().map_err(|error| format!("concurrency: {error}"))?;
    emit(out, &format!(
        "{{\"event\":\"VERIFY\",\"language\":\"rust\",\"ok\":true,\"cases\":{},\"checkpoints\":{},\"concurrency_ok\":true}}",
        lines.len(), checkpoints
    ))
}

fn check_stream_snapshot(replay: &Replay, previous: &mut i64, final_total: i64) -> Result<()> {
    let (got, total) = replay.snapshot();
    if total < *previous || total < 0 || total > final_total || total % 4096 != 0
        || got.len() as i64 != total.min(LIMIT as i64)
    {
        return Err("inconsistent snapshot length/total".into());
    }
    *previous = total;
    let start = total - got.len() as i64;
    if got.iter().enumerate().any(|(index, byte)| *byte != ((start + index as i64) % 251) as u8) {
        return Err("snapshot bytes do not match the stream offset".into());
    }
    Ok(())
}

fn verify_concurrent() -> Result<()> {
    const CHUNK_SIZE: usize = 4096;
    const OPERATIONS: usize = 4096;
    const FINAL_TOTAL: i64 = (CHUNK_SIZE * OPERATIONS) as i64;
    let replay = Arc::new(Replay::default());
    let finished = Arc::new(AtomicBool::new(false));
    let (first_appended_tx, first_appended_rx) = mpsc::sync_channel(0);
    let (first_read_tx, first_read_rx) = mpsc::sync_channel(0);
    let writer_replay = Arc::clone(&replay);
    let writer_finished = Arc::clone(&finished);
    let writer = thread::spawn(move || -> Result<()> {
        let mut input = vec![0; CHUNK_SIZE];
        for operation in 0..OPERATIONS {
            for (index, byte) in input.iter_mut().enumerate() {
                *byte = ((operation * CHUNK_SIZE + index) % 251) as u8;
            }
            writer_replay.append(&input);
            if operation == 0 {
                first_appended_tx.send(()).map_err(|error| error.to_string())?;
                first_read_rx.recv().map_err(|error| error.to_string())?;
            }
            if operation % 16 == 0 {
                thread::yield_now();
            }
        }
        writer_finished.store(true, Ordering::SeqCst);
        Ok(())
    });
    first_appended_rx.recv().map_err(|error| error.to_string())?;
    let mut previous = 0;
    // Guaranteed partial-stream snapshot while the writer waits for this ack.
    let mut failure = check_stream_snapshot(&replay, &mut previous, FINAL_TOTAL).err();
    first_read_tx.send(()).map_err(|error| error.to_string())?;
    while !finished.load(Ordering::SeqCst) && !writer.is_finished() {
        if let Err(error) = check_stream_snapshot(&replay, &mut previous, FINAL_TOTAL) {
            failure = Some(error);
        }
        thread::yield_now();
    }
    writer.join().map_err(|_| "writer thread panicked".to_string())??;
    check_stream_snapshot(&replay, &mut previous, FINAL_TOTAL)?;
    if previous != FINAL_TOTAL {
        return Err("writer did not complete its expected byte count".into());
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn checked_multiply(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b)
        .filter(|value| *value <= i64::MAX as u64)
        .ok_or_else(|| "work count overflows int64".into())
}

// Direct indexing into the mathematical prefill || repeated_work concatenation
// computes only the final retained suffix, independent of buffer operations.
fn expected_snapshot(prefill: &[u8], work: &[u8], work_bytes: u64) -> Vec<u8> {
    let total = prefill.len() as u64 + work_bytes;
    let retained = total.min(LIMIT as u64);
    (0..retained)
        .map(|index| {
            let position = total - retained + index;
            if position < prefill.len() as u64 {
                prefill[position as usize]
            } else {
                work[((position - prefill.len() as u64) % work.len() as u64) as usize]
            }
        })
        .collect()
}

fn measure(options: &Options, input: &mut impl BufRead, out: &mut impl Write) -> Result<()> {
    let fixture = load_fixture(&options.fixture_dir, "mixed.bin", "chunks.csv", false)?;
    let prefill = fs::read(options.fixture_dir.join("prefill.bin")).map_err(|error| error.to_string())?;
    if prefill.len() != LIMIT || fixture.payload.is_empty() {
        return Err("prefill must be exactly 2 MiB and mixed payload must be nonempty".into());
    }
    let (expected_bytes, expected_ops) = if options.mode == "paced" {
        let mut bytes = 0_u64;
        for index in 0..PACED_OPERATIONS {
            bytes = bytes.checked_add(fixture.chunk(index % fixture.boundaries.len()).len() as u64)
                .filter(|value| *value <= i64::MAX as u64)
                .ok_or_else(|| "paced byte count overflows int64".to_string())?;
        }
        (bytes, PACED_OPERATIONS as u64)
    } else {
        (
            checked_multiply(options.repetitions, fixture.payload.len() as u64)?,
            checked_multiply(options.repetitions, fixture.boundaries.len() as u64)?,
        )
    };
    let expected_total = expected_bytes.checked_add(prefill.len() as u64)
        .filter(|value| *value <= i64::MAX as u64)
        .ok_or_else(|| "prefill plus work overflows int64".to_string())?;
    let expected = expected_snapshot(&prefill, &fixture.payload, expected_bytes);
    emit(out, &format!("{{\"event\":\"READY\",\"language\":\"rust\",\"mode\":\"{}\"}}", options.mode))?;
    command(input, "WARMUP")?;
    warmup(&fixture)?;
    let replay = Replay::default();
    replay.append(&prefill);
    emit(out, &format!("{{\"event\":\"WARMED\",\"language\":\"rust\",\"mode\":\"{}\"}}", options.mode))?;
    command(input, "START")?;
    let mut completed_bytes = 0;
    let mut completed_ops = 0;
    let mut max_lateness = Duration::ZERO;
    let start = Instant::now();
    if options.mode == "paced" {
        for index in 0..PACED_OPERATIONS {
            let deadline = start + PACED_INTERVAL * index as u32;
            sleep_until(deadline);
            max_lateness = max_lateness.max(Instant::now().saturating_duration_since(deadline));
            let chunk = fixture.chunk(index % fixture.boundaries.len());
            replay.append(chunk);
            completed_bytes += chunk.len() as u64;
            completed_ops += 1;
        }
        sleep_until(start + PACED_INTERVAL * PACED_OPERATIONS as u32);
    } else {
        for _ in 0..options.repetitions {
            for index in 0..fixture.boundaries.len() {
                let chunk = fixture.chunk(index);
                replay.append(chunk);
                completed_bytes += chunk.len() as u64;
                completed_ops += 1;
            }
        }
    }
    let snapshot_start = Instant::now();
    let (got, total) = replay.snapshot();
    let stop = Instant::now();
    let measurements = Measurements {
        wall_ms: milliseconds(stop.duration_since(start)),
        snapshot_ms: milliseconds(stop.duration_since(snapshot_start)),
        expected_bytes, completed_bytes, expected_ops, completed_ops, total,
        max_lateness_ms: milliseconds(max_lateness),
    };
    let fields = measurements.scalar_fields(&options.mode);
    emit(out, &format!("{{\"event\":\"MEASURED\",{fields}}}"))?;
    command(input, "CHECK")?;
    if completed_bytes != expected_bytes || completed_ops != expected_ops
        || total != expected_total as i64 || got != expected
    {
        return Err("measured output or completed work differs from independent oracle".into());
    }
    emit(out, &format!(
        "{{\"event\":\"DONE\",{fields},\"output_hash\":\"{:016x}\",\"ok\":true}}", fnv1a64(&got)
    ))?;
    let result = command(input, "EXIT");
    // Retain replay, snapshot, fixtures, and oracle through EXIT in both languages.
    black_box(&replay);
    black_box(&got);
    black_box(&prefill);
    black_box(&expected);
    black_box(&fixture);
    result
}

fn warmup(fixture: &Fixture) -> Result<()> {
    let replay = Replay::default();
    let mut total = 0_u64;
    let start = Instant::now();
    loop {
        total = total.checked_add(fixture.payload.len() as u64)
            .filter(|value| *value <= i64::MAX as u64)
            .ok_or_else(|| "warmup total overflows int64".to_string())?;
        for index in 0..fixture.boundaries.len() {
            replay.append(fixture.chunk(index));
        }
        if start.elapsed() >= WARMUP_DURATION {
            return Ok(());
        }
    }
}

fn sleep_until(deadline: Instant) {
    loop {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        thread::sleep(deadline.duration_since(now));
    }
}

fn command(input: &mut impl BufRead, expected: &str) -> Result<()> {
    let mut line = Vec::with_capacity(16);
    loop {
        let buffer = input.fill_buf().map_err(|error| error.to_string())?;
        if buffer.is_empty() {
            return Err(format!("stdin closed before {expected} newline"));
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let length = newline.map_or(buffer.len(), |index| index + 1);
        if line.len() + length > 4096 {
            return Err(format!("overlong command while expecting {expected}"));
        }
        line.extend_from_slice(&buffer[..length]);
        input.consume(length);
        if newline.is_some() {
            break;
        }
    }
    line.pop(); // required newline
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    if line.as_slice() != expected.as_bytes() {
        return Err(format!("expected command {expected}"));
    }
    Ok(())
}

fn emit(out: &mut impl Write, json: &str) -> Result<()> {
    writeln!(out, "{json}").map_err(|error| error.to_string())?;
    out.flush().map_err(|error| error.to_string())
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn fnv1a64(data: &[u8]) -> u64 {
    let mut hash = 14695981039346656037_u64;
    for byte in data {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    hash
}
