use rand::{seq::SliceRandom, thread_rng};
use reqwest::Client;
use warp::{
    http::{Method, Response, StatusCode},
    hyper::Body,
    Filter, Rejection, Reply,
};
use std::convert::Infallible;

static USER_AGENTS: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120",
    "Mozilla/5.0 (X11; Linux x86_64) Gecko/20100101 Firefox/115",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 13_2_1) Safari/605.1.15",
];

#[tokio::main]
async fn main() {
    let client = Client::new();

    let proxy = warp::path("v1")
        .and(warp::path::tail())
        .and(
            warp::query::raw().or_else(|_| async { Ok::<(String,), Infallible>((String::new(),)) }),
        )
        .and(warp::method())
        .and(warp::header::optional::<String>("authorization"))
        .and(warp::header::optional::<String>("if-none-match"))
        .and(warp::body::bytes())
        .and(warp::any().map(move || client.clone()))
        .and_then(handle_proxy);

    println!("Listening on http://0.0.0.0:3030");
    warp::serve(proxy).run(([0, 0, 0, 0], 3030)).await;
}

async fn handle_proxy(
    path: warp::path::Tail,
    query: String,
    method: Method,
    auth: Option<String>,
    etag: Option<String>,
    body: bytes::Bytes,
    client: Client,
) -> Result<impl Reply, Rejection> {
    let mut url = format!("https://api.spotify.com/v1/{}", path.as_str());

    if !query.is_empty() {
        url.push('?');
        url.push_str(&query);
    }

    let ua = USER_AGENTS.choose(&mut thread_rng()).unwrap();

    let mut req = client
        .request(method.clone(), &url)
        .header("User-Agent", *ua);

    if let Some(token) = auth {
        req = req.header("Authorization", token);
    }

    if let Some(tag) = etag {
        req = req.header("If-None-Match", tag);
    }

    if method != Method::GET {
        req = req.body(body);
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(_) => {
            let mut response = Response::new(Body::from("Upstream error"));
            *response.status_mut() = StatusCode::BAD_GATEWAY;
            return Ok(response);
        }
    };

    let mut proxy_resp = Response::builder().status(resp.status());

    for (k, v) in resp.headers() {
        proxy_resp = proxy_resp.header(k, v);
    }

    let stream = resp.bytes_stream();
    Ok(proxy_resp.body(Body::wrap_stream(stream)).unwrap())
}
