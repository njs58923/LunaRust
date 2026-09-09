use js_runtime::{FetchRequest, FetchResponse};
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Client, Method,
};

const MAX_REQUEST: usize = 1024 * 1024;
const MAX_RESPONSE: usize = 8 * 1024 * 1024;

fn validate(request: &FetchRequest) -> Result<(Method, HeaderMap), String> {
    let method =
        Method::from_bytes(request.method.as_bytes()).map_err(|_| "Invalid HTTP method")?;
    if ["CONNECT", "TRACE", "TRACK"]
        .iter()
        .any(|m| request.method.eq_ignore_ascii_case(m))
    {
        return Err("Forbidden HTTP method".into());
    }
    if (request.body.is_some() || request.body_bytes.is_some()) && (method == Method::GET || method == Method::HEAD) {
        return Err("GET/HEAD cannot have a body".into());
    }
    if request
        .body
        .as_ref()
        .is_some_and(|body| body.len() > MAX_REQUEST)
    {
        return Err("Fetch request exceeds 1 MiB".into());
    }
    if request.body_bytes.as_ref().is_some_and(|b| b.len() > MAX_REQUEST) { return Err("Fetch request exceeds 1 MiB".into()); }
    if !matches!(request.redirect.as_str(), "follow" | "error") {
        return Err("Unsupported redirect mode".into());
    }
    if request.headers.len() > 128
        || request
            .headers
            .iter()
            .map(|(k, v)| k.len() + v.len())
            .sum::<usize>()
            > 65536
    {
        return Err("Fetch headers exceed limit".into());
    }
    let mut headers = HeaderMap::new();
    for (name, value) in &request.headers {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| "Invalid header name")?;
        let key = name.as_str();
        if key.starts_with("sec-")
            || key.starts_with("proxy-")
            || matches!(
                key,
                "host"
                    | "content-length"
                    | "connection"
                    | "transfer-encoding"
                    | "trailer"
                    | "te"
                    | "upgrade"
                    | "cookie"
                    | "cookie2"
                    | "origin"
                    | "referer"
                    | "expect"
                    | "accept-encoding"
                    | "access-control-request-method"
                    | "access-control-request-headers"
                    | "keep-alive"
                    | "via"
            )
        {
            return Err(format!("Forbidden request header: {key}"));
        }
        headers.append(
            name,
            HeaderValue::from_str(value).map_err(|_| "Invalid header value")?,
        );
    }
    Ok((method, headers))
}

pub(crate) async fn execute(
    request: FetchRequest,
    origin: Option<String>,
    client: &Client,
) -> Result<FetchResponse, String> {
    let (mut method, mut headers) = validate(&request)?;
    // Native resources keep GET compatibility; remote documents never reach this branch.
    if origin.is_none() && crate::routes::VirtualRoutes::is_virtual_url(&request.url) {
        if method != Method::GET && method != Method::HEAD {
            return Err("Native resources only support GET/HEAD".into());
        }
        let resource = crate::VIRTUAL_ROUTES.resolve(&request.url);
        return Ok(FetchResponse {
            url: request.url,
            status: if resource.is_some() { 200 } else { 404 },
            status_text: if resource.is_some() {
                "OK"
            } else {
                "Not Found"
            }
            .into(),
            body: if method == Method::HEAD {
                Vec::new()
            } else {
                resource.unwrap_or_default().into_bytes()
            },
            headers: vec![],
            redirected: false,
        });
    }
    let parsed = url::Url::parse(&request.url).map_err(|_| "Invalid fetch URL")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Fetch requires HTTP(S)".into());
    }
    // Even a privileged native caller cannot leak auth through cross-origin redirects.
    let origin = origin.unwrap_or_else(|| request.url.clone());
    let mut current = crate::io::same_origin_url(&origin, &request.url)?;
    let mut body = request.body_bytes.or_else(|| request.body.map(String::into_bytes));
    for hop in 0..=5 {
        let mut builder = client
            .request(method.clone(), &current)
            .headers(headers.clone());
        if let Some(body) = &body {
            builder = builder.body(body.clone());
        }
        let mut response = builder
            .send()
            .await
            .map_err(|_| "Fetch network error".to_string())?;
        let status = response.status();
        if matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
            && response.headers().contains_key("location")
        {
            if request.redirect == "error" {
                return Err("Redirect rejected by fetch redirect mode".into());
            }
            if hop == 5 {
                return Err("Too many fetch redirects".into());
            }
            let location = response
                .headers()
                .get("location")
                .unwrap()
                .to_str()
                .map_err(|_| "Invalid redirect Location")?;
            let next = crate::io::same_origin_url(&current, location)?;
            current = crate::io::same_origin_url(&origin, &next)?;
            if ((status.as_u16() == 301 || status.as_u16() == 302) && method == Method::POST)
                || (status.as_u16() == 303 && method != Method::GET && method != Method::HEAD)
            {
                method = Method::GET;
                body = None;
                for name in [
                    "content-type",
                    "content-encoding",
                    "content-language",
                    "content-location",
                ] {
                    headers.remove(name);
                }
            }
            continue;
        }
        let response_headers = response
            .headers()
            .iter()
            .filter(|(name, _)| !matches!(name.as_str(), "set-cookie" | "set-cookie2"))
            .map(|(name, value)| {
                (
                    name.to_string(),
                    value.as_bytes().iter().map(|b| char::from(*b)).collect(),
                )
            })
            .collect();
        if method != Method::HEAD
            && response
                .content_length()
                .is_some_and(|len| len > MAX_RESPONSE as u64)
        {
            return Err("Fetch response exceeds 8 MiB".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "Fetch body read failed")?
        {
            if bytes.len() + chunk.len() > MAX_RESPONSE {
                return Err("Fetch response exceeds 8 MiB".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(FetchResponse {
            url: current,
            status: status.as_u16(),
            status_text: status.canonical_reason().unwrap_or("").into(),
            headers: response_headers,
            body: bytes,
            redirected: hop > 0,
        });
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(url: &str, method: &str) -> FetchRequest {
        FetchRequest {
            url: url.into(),
            method: method.into(),
            headers: vec![],
            body: None,
            body_bytes: None,
            redirect: "follow".into(),
        }
    }

    #[test]
    fn validates_headers_methods_and_request_size_before_sending() {
        let mut r = request("https://test.invalid/", "POST");
        for name in [
            "Host",
            "Cookie",
            "Content-Length",
            "Origin",
            "Connection",
            "Sec-Test",
            "Proxy-Authorization",
        ] {
            r.headers = vec![(name.into(), "x".into())];
            assert!(validate(&r).is_err(), "{name}");
        }
        r.headers = vec![("Authorization".into(), "Bearer key".into())];
        assert!(validate(&r).is_ok());
        r.body = Some("x".repeat(MAX_REQUEST + 1));
        assert!(validate(&r).is_err());
        r.body = Some("x".into());
        r.method = "GET".into();
        assert!(validate(&r).is_err());
        r.body = None;
        r.method = "CONNECT".into();
        assert!(validate(&r).is_err());
    }

    #[test]
    fn real_http_status_headers_body_and_redirect_method_rules() {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let responses = [
                "201 Created\r\nX-Revision: 2\r\nSet-Cookie: private=x\r\nContent-Length: 2\r\n\r\n{}",
                "404 Not Found\r\nContent-Length: 7\r\n\r\nmissing",
                "303 See Other\r\nLocation: /done\r\nContent-Length: 0\r\n\r\n",
                "200 OK\r\nContent-Length: 2\r\n\r\nok",
                "307 Temporary Redirect\r\nLocation: /done\r\nContent-Length: 0\r\n\r\n",
                "200 OK\r\nContent-Length: 2\r\n\r\nok",
                "304 Not Modified\r\nContent-Length: 0\r\n\r\n",
            ];
            let mut seen = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(&mut stream);
                let mut head = String::new();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some(n) = line.to_lowercase().strip_prefix("content-length:") {
                        length = n.trim().parse().unwrap();
                    }
                    head.push_str(&line);
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                seen.push((head, String::from_utf8(body).unwrap()));
                write!(stream, "HTTP/1.1 {response}").unwrap();
            }
            seen
        });
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let client = Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap();
            let mut post = request(&url, "POST");
            post.body = Some("{\"title\":\"door\"}".into());
            post.headers = vec![
                ("content-type".into(), "application/json".into()),
                ("authorization".into(), "Bearer key".into()),
            ];
            let created = execute(post.clone(), Some(url.clone()), &client)
                .await
                .unwrap();
            assert_eq!(created.status, 201);
            assert_eq!(created.body, b"{}");
            assert!(created.headers.contains(&("x-revision".into(), "2".into())));
            assert!(!created.headers.iter().any(|(k, _)| k == "set-cookie"));
            let missing = execute(request(&url, "GET"), Some(url.clone()), &client)
                .await
                .unwrap();
            assert_eq!(missing.status, 404);
            assert_eq!(missing.body, b"missing");
            let redirected = execute(post.clone(), Some(url.clone()), &client)
                .await
                .unwrap();
            assert!(redirected.redirected && redirected.url.ends_with("/done"));
            execute(post, Some(url.clone()), &client).await.unwrap();
            assert_eq!(
                execute(request(&url, "GET"), Some(url.clone()), &client)
                    .await
                    .unwrap()
                    .status,
                304
            );
        });
        let seen = server.join().unwrap();
        assert!(seen[0].0.starts_with("POST / HTTP/1.1"));
        assert!(seen[0]
            .0
            .to_lowercase()
            .contains("authorization: bearer key"));
        assert_eq!(seen[0].1, "{\"title\":\"door\"}");
        assert!(seen[3].0.starts_with("GET /done") && seen[3].1.is_empty());
        assert!(!seen[3].0.to_lowercase().contains("content-type:"));
        assert!(seen[5].0.starts_with("POST /done"));
        assert_eq!(seen[5].1, seen[0].1);
    }
}
