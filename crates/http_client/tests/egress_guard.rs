//! Exercises the process-wide egress policy through the real HTTP client. This is its own test
//! binary because restricting egress cannot be undone within a process.

use std::error::Error as _;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

fn is_blocked(error: &reqwest::Error) -> bool {
    let mut source: Option<&(dyn std::error::Error + 'static)> = error.source();
    while let Some(current) = source {
        if current.downcast_ref::<egress_guard::Blocked>().is_some() {
            return true;
        }
        source = current.source();
    }
    false
}

#[test]
fn restricted_client_only_reaches_the_allowlisted_endpoint() {
    let endpoint = TcpListener::bind("127.0.0.1:0").unwrap();
    let allowed = url::Url::parse(&format!(
        "http://127.0.0.1:{}/v1/models",
        endpoint.local_addr().unwrap().port()
    ))
    .unwrap();
    // A second local port that must never see a connection.
    let other = TcpListener::bind("127.0.0.1:0").unwrap();
    other.set_nonblocking(true).unwrap();
    let other_url = format!("http://127.0.0.1:{}/", other.local_addr().unwrap().port());

    thread::spawn(move || {
        let (mut stream, _) = endpoint.accept().unwrap();
        let mut request = [0u8; 2048];
        let _ = stream.read(&mut request);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .unwrap();
    });

    egress_guard::restrict();
    egress_guard::set_allowed_urls([&allowed]);
    let client = http_client::Client::new();

    futures::executor::block_on(async {
        let response = client
            .get(allowed.as_str())
            .send()
            .await
            .expect("the allowlisted endpoint is reachable");
        assert_eq!(response.text().await.unwrap(), "ok");

        let Err(error) = client.get(other_url.as_str()).send().await else {
            panic!("another port on the same host must be refused");
        };
        assert!(is_blocked(&error), "{error:?}");

        let Err(error) = client
            .post("https://app.warp.dev/ai/multi-agent")
            .body("secret")
            .send()
            .await
        else {
            panic!("Warp's server must be refused");
        };
        assert!(is_blocked(&error), "{error:?}");
    });

    assert!(
        other.accept().is_err(),
        "the refused port never received a connection"
    );
    let attempts = egress_guard::recent_blocked_attempts();
    assert!(
        attempts
            .iter()
            .any(|attempt| attempt.target == "https://app.warp.dev")
    );
}
