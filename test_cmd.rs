use std::process::{Command, Stdio};
use std::io::{BufReader, BufRead, Write, Read};
use std::thread;

fn main() {
    let mut child = Command::new("cmd.exe")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdout = child.stdout.take().unwrap();
    let mut stdin = child.stdin.take().unwrap();

    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 { break; }
            print!("{}", String::from_utf8_lossy(&buf[..n]));
        }
    });

    stdin.write_all(b"dir\n").unwrap();
    thread::sleep(std::time::Duration::from_millis(500));
}
