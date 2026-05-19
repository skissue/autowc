use std::io::{self, Write};

pub fn send(line: impl AsRef<str>) {
    let mut stdout = io::stdout().lock();
    let _ = writeln!(stdout, "{}", line.as_ref());
    let _ = stdout.flush();
}
