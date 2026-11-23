mod options;
mod payload;
use http_body_util::BodyExt;
pub use options::*;
pub use payload::{PayloadSnapshot, is_cookie_header};
use reqwest::{
    Client, Method, Url,
    header::{self, HeaderMap, HeaderName, HeaderValue},
};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct ResponseCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub expires: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: String,
    pub raw: String,
}
#[derive(Default, Clone)]
pub struct HttpResponse {
    pub request_payload: Option<PayloadSnapshot>,
    pub sent_at: u64,
    pub status: u16,
    pub body: String,
    pub headers: Vec<(String, String)>,
    pub cookies: Vec<ResponseCookie>,
    pub elapsed_ms: u128,
    pub size: usize,
}

fn http_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value.trim()).map_err(|e| format!("Invalid URL: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Only HTTP and HTTPS URLs are supported".into());
    }
    Ok(url)
}

async fn oauth_token(auth: &Auth) -> Result<String, String> {
    if auth.oauth_flow == OAuthFlow::AccessToken {
        return if auth.token.trim().is_empty() {
            Err("Access token is required".into())
        } else {
            Ok(auth.token.clone())
        };
    }
    let url = http_url(&auth.token_url)?;
    // Do not forward client credentials through redirects.
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let mut form = vec![("grant_type", "client_credentials")];
    if !auth.scope.is_empty() {
        form.push(("scope", &auth.scope));
    }
    let response = client
        .post(url)
        .basic_auth(&auth.client_id, Some(&auth.client_secret))
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("Token request failed: {}", e.without_url()))?;
    if !response.status().is_success() {
        return Err(format!(
            "Token endpoint returned HTTP {}",
            response.status().as_u16()
        ));
    }
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|_| "Token endpoint did not return valid JSON")?;
    value
        .get("access_token")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "Token response has no access_token".into())
}

/// Build a request independently of transport, so validation happens before sending.
pub fn build_request(
    client: &Client,
    url: &str,
    method: Method,
    options: &RequestOptions,
    oauth_token: Option<&str>,
) -> Result<reqwest::Request, String> {
    let mut url = http_url(url)?;
    if options
        .params
        .iter()
        .any(|p| p.enabled && !p.name.trim().is_empty())
    {
        url.query_pairs_mut()
            .extend_pairs(enabled_pairs(&options.params));
    }
    let mut headers = HeaderMap::new();
    for (name, value) in enabled_pairs(&options.headers) {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| "Invalid header name")?;
        let value = HeaderValue::from_str(value)
            .map_err(|_| "Invalid header value (newlines are not allowed)")?;
        headers.append(name, value);
    }
    let cookies: Vec<_> = enabled_pairs(&options.cookies)
        .map(|(name, value)| {
            if name
                .bytes()
                .any(|c| !c.is_ascii_alphanumeric() && !b"!#$%&'*+-.^_`|~".contains(&c))
                || value
                    .bytes()
                    .any(|c| c <= 0x20 || c >= 0x7f || b"\";,\\".contains(&c))
            {
                return Err("Invalid cookie name or value".to_string());
            }
            Ok(format!("{name}={value}"))
        })
        .collect::<Result<_, _>>()?;
    if !cookies.is_empty() {
        if headers.contains_key(header::COOKIE) {
            return Err("Use either the Cookies tab or a Cookie header".into());
        }
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&cookies.join("; ")).map_err(|_| "Invalid cookies")?,
        );
    }
    let auth = &options.auth;
    let generates_authorization = matches!(
        auth.kind,
        AuthKind::Basic | AuthKind::Bearer | AuthKind::OAuth2
    ) || (auth.kind == AuthKind::ApiKey
        && auth.key_location == KeyLocation::Header
        && auth.key_name.eq_ignore_ascii_case("authorization"));
    if generates_authorization && headers.contains_key(header::AUTHORIZATION) {
        return Err("Remove the Authorization header or select No auth".into());
    }
    let mut request = client.request(method, url).headers(headers);
    request = match auth.kind {
        AuthKind::None => request,
        AuthKind::Basic => request.basic_auth(&auth.username, Some(&auth.password)),
        AuthKind::Bearer => {
            if auth.token.trim().is_empty() {
                return Err("Bearer token is required".into());
            }
            request.bearer_auth(&auth.token)
        }
        AuthKind::OAuth2 => {
            request.bearer_auth(oauth_token.ok_or("OAuth access token is missing")?)
        }
        AuthKind::ApiKey => {
            if auth.key_name.trim().is_empty() {
                return Err("API key name is required".into());
            }
            match auth.key_location {
                KeyLocation::Header => {
                    let name = HeaderName::from_bytes(auth.key_name.as_bytes())
                        .map_err(|_| "Invalid API key header name")?;
                    let mut value = HeaderValue::from_str(&auth.key_value)
                        .map_err(|_| "Invalid API key header value")?;
                    value.set_sensitive(true);
                    request.header(name, value)
                }
                KeyLocation::Query => request.query(&[(&auth.key_name, &auth.key_value)]),
            }
        }
    };
    // Explicit Content-Type takes precedence for raw bodies. Form encoders own theirs.
    request = match options.body.kind {
        BodyKind::None => request,
        BodyKind::Json => {
            serde_json::from_str::<serde_json::Value>(&options.body.text)
                .map_err(|e| format!("Invalid JSON body: {e}"))?;
            request.body(options.body.text.clone())
        }
        BodyKind::Text => request.body(options.body.text.clone()),
        BodyKind::Form => request.form(&enabled_pairs(&options.body.fields).collect::<Vec<_>>()),
        BodyKind::Multipart => {
            if options
                .headers
                .iter()
                .any(|p| p.enabled && p.name.eq_ignore_ascii_case("content-type"))
            {
                return Err(
                    "Remove the Content-Type header: multipart generates its own boundary".into(),
                );
            }
            let mut form = reqwest::multipart::Form::new();
            for (name, value) in enabled_pairs(&options.body.fields) {
                form = form.text(name.to_owned(), value.to_owned());
            }
            request.multipart(form)
        }
    };
    let mut request = request
        .build()
        .map_err(|e| format!("Invalid request: {e}"))?;
    if !request.headers().contains_key(header::CONTENT_TYPE) {
        let content_type = match options.body.kind {
            BodyKind::Json => Some("application/json"),
            BodyKind::Text => Some("text/plain; charset=utf-8"),
            _ => None,
        };
        if let Some(value) = content_type {
            request
                .headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(value));
        }
    }
    Ok(request)
}

pub async fn send_request_with_options(
    url: &str,
    method: Method,
    options: RequestOptions,
) -> Result<HttpResponse, String> {
    // Expose redirects as responses, preserving their headers and Set-Cookie values.
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let token = if options.auth.kind == AuthKind::OAuth2 {
        Some(oauth_token(&options.auth).await?)
    } else {
        None
    };
    let mut request = build_request(&client, url, method, &options, token.as_deref())?;
    // Multipart bodies are streams. Materialize the existing text-only body once
    // so the snapshot uses the same encoded bytes and boundary as the sent body.
    let body = if let Some(body) = request.body_mut().take() {
        let bytes = body
            .collect()
            .await
            .map_err(|_| "Could not prepare request body")?
            .to_bytes();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        *request.body_mut() = Some(reqwest::Body::from(bytes));
        text
    } else {
        String::new()
    };
    let request_payload = PayloadSnapshot::new(
        body,
        request.headers().iter().map(|(name, value)| {
            (
                name.to_string(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        }),
    );
    let sent_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let started = Instant::now();
    let response = client
        .execute(request)
        .await
        .map_err(|e| e.without_url().to_string())?;
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.to_string(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect();
    let cookies = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| {
            let raw = value.to_str().ok()?;
            let c = cookie::Cookie::parse(raw).ok()?;
            Some(ResponseCookie {
                name: c.name().into(),
                value: c.value().into(),
                domain: c
                    .domain()
                    .unwrap_or(response.url().host_str().unwrap_or(""))
                    .into(),
                path: c
                    .path()
                    .filter(|p| p.starts_with('/'))
                    .unwrap_or_else(|| {
                        let path = response.url().path();
                        path.rfind('/')
                            .filter(|index| *index > 0)
                            .map_or("/", |index| &path[..index])
                    })
                    .into(),
                expires: c
                    .max_age()
                    .map(|age| format!("Max-Age: {}s", age.whole_seconds()))
                    .or_else(|| c.expires_datetime().map(|d| d.to_string()))
                    .unwrap_or_else(|| "Session".into()),
                secure: c.secure().unwrap_or(false),
                http_only: c.http_only().unwrap_or(false),
                same_site: c.same_site().map(|s| s.to_string()).unwrap_or_default(),
                raw: raw.into(),
            })
        })
        .collect();
    let bytes = response
        .bytes()
        .await
        .map_err(|e| e.without_url().to_string())?;
    Ok(HttpResponse {
        request_payload: Some(request_payload),
        sent_at,
        status,
        body: String::from_utf8_lossy(&bytes).into_owned(),
        headers,
        cookies,
        elapsed_ms: started.elapsed().as_millis(),
        size: bytes.len(),
    })
}

pub async fn send_request(url: &str, method: Method) -> Result<HttpResponse, String> {
    send_request_with_options(url, method, RequestOptions::default()).await
}

#[cfg(test)]
mod tests;

impl std::fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("size", &self.size)
            .field("elapsed_ms", &self.elapsed_ms)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for ResponseCookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ResponseCookie(<redacted>)")
    }
}
