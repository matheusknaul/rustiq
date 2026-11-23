use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// A body and prepared headers only. Cookies never enter this snapshot.
#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadSnapshot {
    pub body: String,
    #[serde(serialize_with = "headers_without_cookies")]
    pub headers: Vec<(String, String)>,
}

pub fn is_cookie_header(name: &str) -> bool {
    ["cookie", "set-cookie", "cookie2", "set-cookie2"]
        .iter()
        .any(|cookie| name.eq_ignore_ascii_case(cookie))
}
impl PayloadSnapshot {
    pub fn new(body: String, headers: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            body,
            headers: headers
                .into_iter()
                .filter(|(name, _)| !is_cookie_header(name))
                .collect(),
        }
    }
    pub fn byte_len(&self) -> usize {
        self.body.len().saturating_add(
            self.headers
                .iter()
                .map(|(k, v)| k.len().saturating_add(v.len()))
                .sum::<usize>(),
        )
    }
    pub fn exclude_cookies(&mut self) {
        self.headers.retain(|(name, _)| !is_cookie_header(name));
    }
}
impl std::fmt::Debug for PayloadSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PayloadSnapshot(<redacted>)")
    }
}
impl Drop for PayloadSnapshot {
    fn drop(&mut self) {
        self.body.zeroize();
        for (name, value) in &mut self.headers {
            name.zeroize();
            value.zeroize();
        }
    }
}

fn headers_without_cookies<S: serde::Serializer>(
    headers: &[(String, String)],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(headers.iter().filter(|(name, _)| !is_cookie_header(name)))
}
