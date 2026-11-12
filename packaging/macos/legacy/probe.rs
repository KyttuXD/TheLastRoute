//! Standalone Snow Leopard toolchain probe, not the game or a compatibility claim.
use std::{cell::Cell, fs, io::{Read, Write}, net::{TcpListener, TcpStream}, thread};

thread_local! { static VALUE: Cell<u32> = Cell::new(0); }

fn main() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(std::env::consts::OS, "macos", "Run this probe on macOS");
    assert_eq!(std::env::consts::ARCH, "x86_64", "Intel x86_64 probe only");
    VALUE.with(|v| v.set(42));
    let worker = thread::spawn(|| {
        VALUE.with(|v| {
            assert_eq!(v.get(), 0);
            v.set(7);
        });
        let data: Vec<u32> = (0..4096).collect();
        assert_eq!(data.iter().sum::<u32>(), 8_386_560);
    });
    worker.join().expect("worker panicked");
    VALUE.with(|v| assert_eq!(v.get(), 42));
    let dir = std::env::temp_dir().join(format!("last-route-runtime-probe-{}", std::process::id()));
    fs::create_dir(&dir)?;
    let file = dir.join("probe.txt");
    fs::write(&file, b"Last Route runtime probe")?;
    assert_eq!(fs::read(&file)?, b"Last Route runtime probe");
    fs::remove_file(&file)?;
    fs::remove_dir(&dir)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let mut client = TcpStream::connect(listener.local_addr()?)?;
    let (mut server, _) = listener.accept()?;
    client.write_all(b"LR")?;
    let mut message = [0; 2];
    server.read_exact(&mut message)?;
    assert_eq!(&message, b"LR");
    println!("PASS: Intel macOS allocation, threads, TLS isolation, files and loopback TCP");
    println!("This tests the compiler runtime only, not game or graphics compatibility.");
    Ok(())
}
