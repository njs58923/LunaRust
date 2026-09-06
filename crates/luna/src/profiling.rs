//! Opt-in CPU phase trace for reproducing streaming stalls.
//! Set LUNA_PROFILE_CSV to a local file before starting Luna. No work or file
//! access after initialization when unset. Buffered writes avoid per-span I/O.
use std::{fs::File, io::{BufWriter, Write}, sync::{Mutex, OnceLock}, time::Instant};

struct Recorder { start: Instant, output: Mutex<BufWriter<File>> }
static RECORDER: OnceLock<Option<Recorder>> = OnceLock::new();

pub(crate) struct Span { recorder: &'static Recorder, label: &'static str, start: Instant }

pub(crate) fn span(label: &'static str) -> Option<Span> {
    let recorder = RECORDER.get_or_init(|| {
        let path = std::env::var_os("LUNA_PROFILE_CSV")?;
        match File::create(path) {
            Ok(file) => {
                let mut output = BufWriter::new(file);
                let _ = writeln!(output, "elapsed_ms,phase,duration_ms");
                Some(Recorder { start: Instant::now(), output: Mutex::new(output) })
            }
            Err(error) => { eprintln!("Luna profiler could not open output: {error}"); None }
        }
    }).as_ref()?;
    Some(Span { recorder, label, start: Instant::now() })
}

impl Drop for Span {
    fn drop(&mut self) {
        let duration = self.start.elapsed().as_secs_f64() * 1000.0;
        if let Ok(mut output) = self.recorder.output.lock() {
            let _ = writeln!(output, "{:.3},{},{:.3}", self.start.duration_since(self.recorder.start).as_secs_f64()*1000.0, self.label, duration);
        }
    }
}
