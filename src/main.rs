use reqwest::{Client, Method};
use rand::rng;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task;
use tokio::time::timeout;
use rand::prelude::IndexedRandom;

static USER_AGENTS: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120",
    "Mozilla/5.0 (X11; Linux x86_64) Gecko/20100101 Firefox/115",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 13_2_1) Safari/605.1.15",
];

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("0.0.0.0:3030").await?;
    let client = Client::new();

    println!("Listening on http://0.0.0.0:3030");

    loop {
        let (mut socket, _) = listener.accept().await?;
        let client = client.clone();

        task::spawn(async move {
            let mut buffer = [0; 8192];
            let mut req_data = Vec::new();

            match timeout(std::time::Duration::from_secs(5), socket.read(&mut buffer)).await {
                Ok(Ok(n)) if n > 0 => {
                    req_data.extend_from_slice(&buffer[..n]);
                }
                _ => return,
            }

            let request_text = match std::str::from_utf8(&req_data) {
                Ok(text) => text,
                Err(_) => return,
            };

            if !request_text.starts_with("GET") && !request_text.starts_with("POST")
                && !request_text.starts_with("PUT") && !request_text.starts_with("DELETE")
                && !request_text.starts_with("PATCH")
            {
                let _ = socket.write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n").await;
                return;
            }

            let lines: Vec<&str> = request_text.split("\r\n").collect();
            let (method_str, path) = {
                let parts: Vec<&str> = lines[0].split_whitespace().collect();
                if parts.len() < 2 {
                    let _ = socket.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
                    return;
                }
                (parts[0], parts[1])
            };

            let method: Method = method_str.parse().unwrap_or(Method::GET);
            let mut auth_header = None;
            let mut etag_header = None;

            for line in &lines[1..] {
                if line.to_lowercase().starts_with("authorization:") {
                    auth_header = Some(line["authorization:".len()..].trim().to_string());
                } else if line.to_lowercase().starts_with("if-none-match:") {
                    etag_header = Some(line["if-none-match:".len()..].trim().to_string());
                }
            }

            let url = format!("https://api.spotify.com{}", path);

            let user_agent = USER_AGENTS.choose(&mut rng()).unwrap();


            let mut req_builder = client.request(method.clone(), &url).header("User-Agent", *user_agent);

            if let Some(token) = auth_header {
                req_builder = req_builder.header("Authorization", token);
            }

            if let Some(etag) = etag_header {
                req_builder = req_builder.header("If-None-Match", etag);
            }

            if method != Method::GET {
                if let Some(body_start) = request_text.find("\r\n\r\n") {
                    let body = &req_data[body_start + 4..];
                    req_builder = req_builder.body(body.to_vec());
                }
            }

            match req_builder.send().await {
                Ok(mut resp) => {
                    let status_line = format!(
                        "HTTP/1.1 {} {}\r\n",
                        resp.status().as_u16(),
                        resp.status().canonical_reason().unwrap_or("OK")
                    );
                    let mut headers = String::new();
                    for (key, value) in resp.headers() {
                        if key != "transfer-encoding" && key != "content-length" {
                            headers.push_str(&format!("{}: {}\r\n", key, value.to_str().unwrap_or("")));
                        }
                    }

                    let _ = socket.write_all(status_line.as_bytes()).await;
                    let _ = socket.write_all(headers.as_bytes()).await;
                    let _ = socket.write_all(b"\r\n").await;

                    while let Some(chunk) = resp.chunk().await.unwrap_or(None) {
                        let _ = socket.write_all(&chunk).await;
                    }
                }
                Err(e) => {
                    eprintln!("Upstream error: {}", e);
                    let _ = socket.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
                }
            }
        });
    }
}
