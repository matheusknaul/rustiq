use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

fn pair(name: &str, value: &str) -> Pair {
    Pair {
        name: name.into(),
        value: value.into(),
        enabled: true,
    }
}
fn build(options: &RequestOptions) -> reqwest::Request {
    build_request(
        &Client::new(),
        "http://localhost/api?existing=1",
        Method::POST,
        options,
        Some("oauth-token"),
    )
    .unwrap()
}

#[test]
fn query_headers_and_cookies_preserve_duplicates_and_ignore_disabled_rows() {
    let mut options = RequestOptions::default();
    options.params = vec![
        pair("q", "a & b"),
        pair("tag", "one"),
        pair("tag", "two"),
        Pair {
            enabled: false,
            ..pair("secret", "ignored")
        },
    ];
    options.headers = vec![pair("x-test", "one"), pair("x-test", "two")];
    options.cookies = vec![pair("session", "abc"), pair("theme", "dark")];
    let request = build(&options);
    assert_eq!(
        request.url().query_pairs().collect::<Vec<_>>(),
        vec![
            ("existing".into(), "1".into()),
            ("q".into(), "a & b".into()),
            ("tag".into(), "one".into()),
            ("tag".into(), "two".into())
        ]
    );
    assert_eq!(request.headers().get_all("x-test").iter().count(), 2);
    assert_eq!(request.headers()["cookie"], "session=abc; theme=dark");
}

#[test]
fn auth_modes_and_conflicting_headers() {
    let mut options = RequestOptions::default();
    assert!(!build(&options).headers().contains_key("authorization"));
    options.auth.kind = AuthKind::Basic;
    options.auth.username = "user".into();
    options.auth.password = "pass".into();
    assert_eq!(
        build(&options).headers()["authorization"],
        "Basic dXNlcjpwYXNz"
    );
    options.auth.kind = AuthKind::Bearer;
    options.auth.token = "token".into();
    assert_eq!(build(&options).headers()["authorization"], "Bearer token");
    options.auth.kind = AuthKind::OAuth2;
    assert_eq!(
        build(&options).headers()["authorization"],
        "Bearer oauth-token"
    );
    options.auth.kind = AuthKind::ApiKey;
    options.auth.key_name = "x-api-key".into();
    options.auth.key_value = "secret".into();
    assert_eq!(build(&options).headers()["x-api-key"], "secret");
    options.auth.key_location = KeyLocation::Query;
    assert!(
        build(&options)
            .url()
            .query_pairs()
            .any(|(k, v)| k == "x-api-key" && v == "secret")
    );
    options.auth.kind = AuthKind::Basic;
    options.headers.push(pair("Authorization", "manual"));
    assert!(
        build_request(
            &Client::new(),
            "http://localhost",
            Method::GET,
            &options,
            None
        )
        .is_err()
    );
}

#[test]
fn body_encodings_and_validation() {
    let mut options = RequestOptions::default();
    options.body.kind = BodyKind::Json;
    options.body.text = "{\"ok\":true}".into();
    let request = build(&options);
    assert_eq!(request.headers()["content-type"], "application/json");
    assert_eq!(
        request.body().unwrap().as_bytes().unwrap(),
        b"{\"ok\":true}"
    );
    options.body.text = "bad json".into();
    assert!(
        build_request(
            &Client::new(),
            "http://localhost",
            Method::GET,
            &options,
            None
        )
        .is_err()
    );
    options.body.kind = BodyKind::Text;
    assert_eq!(
        build(&options).headers()["content-type"],
        "text/plain; charset=utf-8"
    );
    options.body.kind = BodyKind::Form;
    options.body.fields = vec![pair("q", "hello world&"), pair("q", "two")];
    let request = build(&options);
    assert_eq!(
        request.headers()["content-type"],
        "application/x-www-form-urlencoded"
    );
    assert_eq!(
        request.body().unwrap().as_bytes().unwrap(),
        b"q=hello+world%26&q=two"
    );
    options.body.kind = BodyKind::Multipart;
    assert!(
        build(&options).headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("multipart/form-data; boundary=")
    );
    options
        .headers
        .push(pair("Content-Type", "application/json"));
    assert!(
        build_request(
            &Client::new(),
            "http://localhost",
            Method::GET,
            &options,
            None
        )
        .is_err()
    );
}

#[test]
fn invalid_values_fail_before_network_io() {
    let mut options = RequestOptions::default();
    options.headers = vec![pair("x-test", "value\r\ninjected: yes")];
    assert!(
        build_request(
            &Client::new(),
            "http://localhost",
            Method::GET,
            &options,
            None
        )
        .is_err()
    );
    options.headers.clear();
    options.cookies = vec![pair("session", "abc; injected=yes")];
    assert!(
        build_request(
            &Client::new(),
            "http://localhost",
            Method::GET,
            &options,
            None
        )
        .is_err()
    );
    assert!(
        build_request(
            &Client::new(),
            "file:///etc/passwd",
            Method::GET,
            &RequestOptions::default(),
            None
        )
        .is_err()
    );
}

// One-shot HTTP server: tests inspect the actual wire request without external services.
fn server(response: String) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let count = stream.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                let length = headers
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("content-length:")
                            .and_then(|n| n.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if headers.contains("transfer-encoding: chunked") {
                    if bytes.ends_with(b"0\r\n\r\n") {
                        break;
                    }
                } else if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }
        stream.write_all(response.as_bytes()).unwrap();
        String::from_utf8(bytes).unwrap()
    });
    (format!("http://{address}"), handle)
}
fn reply(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[tokio::test]
async fn captures_response_headers_cookies_body_and_redirects() {
    let (url, handle) = server("HTTP/1.1 302 Found\r\nLocation: /next\r\nSet-Cookie: session=abc; Path=/; HttpOnly; Secure; SameSite=Lax\r\nSet-Cookie: theme=dark; Max-Age=60\r\nX-Test: one\r\nX-Test: two\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK".into());
    let result = send_request_with_options(&url, Method::GET, RequestOptions::default())
        .await
        .unwrap();
    assert_eq!(result.status, 302);
    assert_eq!(result.body, "OK");
    assert_eq!(result.size, 2);
    assert_eq!(
        result.headers.iter().filter(|(k, _)| k == "x-test").count(),
        2
    );
    assert_eq!(result.cookies.len(), 2);
    assert_eq!(result.cookies[0].name, "session");
    assert!(result.cookies[0].http_only);
    assert!(result.cookies[0].secure);
    assert_eq!(result.cookies[0].same_site, "Lax");
    handle.join().unwrap();
}

#[tokio::test]
async fn oauth_client_credentials_fetches_and_uses_access_token() {
    let (token_url, token_server) = server(reply(
        "{\"access_token\":\"generated\",\"token_type\":\"Bearer\"}",
    ));
    let (url, api_server) = server(reply("done"));
    let mut options = RequestOptions::default();
    options.auth = Auth {
        kind: AuthKind::OAuth2,
        oauth_flow: OAuthFlow::ClientCredentials,
        token_url,
        client_id: "user".into(),
        client_secret: "pass".into(),
        scope: "read write".into(),
        ..Default::default()
    };
    assert_eq!(
        send_request_with_options(&url, Method::GET, options)
            .await
            .unwrap()
            .body,
        "done"
    );
    let token_request = token_server.join().unwrap();
    assert!(
        token_request
            .to_lowercase()
            .contains("authorization: basic dxnlcjpwyxnz")
    );
    assert!(token_request.contains("grant_type=client_credentials&scope=read+write"));
    assert!(api_server.join().unwrap().contains("Bearer generated"));
}

#[tokio::test]
async fn multipart_fields_are_sent_with_the_matching_boundary() {
    let (url, handle) = server(reply("ok"));
    let mut options = RequestOptions::default();
    options.body.kind = BodyKind::Multipart;
    options.body.fields = vec![pair("field", "hello multipart")];
    send_request_with_options(&url, Method::POST, options)
        .await
        .unwrap();
    let raw = handle.join().unwrap();
    let boundary = raw
        .lines()
        .find_map(|l| l.strip_prefix("content-type: multipart/form-data; boundary="))
        .unwrap();
    assert!(raw.contains(&format!("--{boundary}\r\n")));
    assert!(raw.contains("name=\"field\"\r\n\r\nhello multipart"));
}

#[test]
fn secret_values_are_not_exposed_by_debug_output() {
    let mut options = RequestOptions::default();
    options.auth.token = "token-secret".into();
    options.headers.push(pair("secret-header", "header-secret"));
    options.body.text = "body-secret".into();
    assert!(!format!("{options:?}").contains("-secret"));
    let response = HttpResponse {
        body: "response-secret".into(),
        ..Default::default()
    };
    assert!(!format!("{response:?}").contains("response-secret"));
}
