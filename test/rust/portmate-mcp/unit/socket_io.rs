use super::*;
use std::io::Write;
use std::net::TcpListener;

fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let server = listener.accept().unwrap().0;
    (client, server)
}

#[test]
fn timeout_setup_fallback_accepts_delayed_data_and_restores_blocking_io() {
    let (mut client, mut server) = pair();
    let writer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        server.write_all(b"data").unwrap();
        std::thread::sleep(Duration::from_millis(30));
        server.write_all(b"later").unwrap();
    });
    let mut buffer = [0; 4];
    assert_eq!(
        read_nonblocking_before(
            &mut client,
            &mut buffer,
            Instant::now() + Duration::from_secs(1),
            "deadline"
        )
        .unwrap(),
        4
    );
    assert_eq!(&buffer, b"data");
    let mut later = [0; 5];
    client.read_exact(&mut later).unwrap();
    assert_eq!(&later, b"later");
    writer.join().unwrap();
}

#[test]
fn timeout_setup_fallback_still_bounds_silent_connections() {
    let (mut client, _server) = pair();
    let started = Instant::now();
    let error = read_nonblocking_before(
        &mut client,
        &mut [0],
        started + Duration::from_millis(30),
        "absolute deadline",
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert_eq!(error.to_string(), "absolute deadline");
    assert!(started.elapsed() >= Duration::from_millis(25));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn timeout_setup_fallback_preserves_eof() {
    let (mut client, server) = pair();
    drop(server);
    assert_eq!(
        read_nonblocking_before(
            &mut client,
            &mut [0],
            Instant::now() + Duration::from_secs(1),
            "deadline"
        )
        .unwrap(),
        0
    );
}
