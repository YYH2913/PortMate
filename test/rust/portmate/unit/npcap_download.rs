use super::*;

async fn response(header: &str, body: &[u8]) -> reqwest::Response {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let header = header.as_bytes().to_vec();
    let body = body.to_vec();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 8192];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        socket.write_all(&header).await.unwrap();
        socket.write_all(&body).await.unwrap();
    });
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(format!("http://{address}/installer"))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn verifies_all_downloaded_bytes_and_reports_progress_without_running_anything() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("installer.exe");
    let bytes = b"local test data, not an installer";
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    let mut progress = Vec::new();
    receive(
        response(&header, bytes).await,
        &path,
        &hex_hash(&Sha256::digest(bytes)),
        &AtomicBool::new(false),
        |done, total| progress.push((done, total)),
    )
    .await
    .unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(progress.first(), Some(&(0, Some(bytes.len() as u64))));
    assert_eq!(
        progress.last(),
        Some(&(bytes.len() as u64, Some(bytes.len() as u64)))
    );
    assert_eq!(INSTALLER_SHA256.len(), 64);
    assert_eq!(DOWNLOAD_URL, "https://npcap.com/dist/npcap-1.90.exe");
}

#[tokio::test]
async fn rejects_redirects_http_errors_oversized_empty_and_changed_content() {
    for (header, body) in [
        (
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/never\r\nContent-Length: 0\r\n\r\n",
            &b""[..],
        ),
        (
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n",
            &b""[..],
        ),
        (
            "HTTP/1.1 200 OK\r\nContent-Length: 33554433\r\n\r\n",
            &b""[..],
        ),
        ("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n", &b""[..]),
        ("HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n", &b"evil"[..]),
    ] {
        let root = tempfile::tempdir().unwrap();
        assert!(receive(
            response(header, body).await,
            &root.path().join("setup.exe"),
            INSTALLER_SHA256,
            &AtomicBool::new(false),
            |_, _| {}
        )
        .await
        .is_err());
    }
}

#[tokio::test]
async fn cancellation_interrupts_a_stalled_body_and_refuses_to_overwrite_existing_files() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0_u8; 8192];
        assert!(socket.read(&mut buffer).await.unwrap() > 0);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
    });
    let stalled_response = reqwest::get(format!("http://{address}/")).await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("setup.exe");
    let cancelled = AtomicBool::new(false);
    let work = receive(
        stalled_response,
        &path,
        INSTALLER_SHA256,
        &cancelled,
        |_, _| {},
    );
    tokio::pin!(work);
    tokio::select! {
        _ = &mut work => panic!("stalled body completed before cancellation"),
        _ = tokio::time::sleep(Duration::from_millis(25)) => cancelled.store(true, Ordering::SeqCst),
    }
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), work)
            .await
            .unwrap()
            .unwrap_err()
            .code,
        "cancelled"
    );
    server.abort();
    let path = root.path().join("existing.exe");
    fs::write(&path, b"keep me").unwrap();
    let header = "HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n";
    assert!(receive(
        response(header, b"data").await,
        &path,
        INSTALLER_SHA256,
        &AtomicBool::new(false),
        |_, _| {}
    )
    .await
    .is_err());
    assert_eq!(fs::read(path).unwrap(), b"keep me");
}
