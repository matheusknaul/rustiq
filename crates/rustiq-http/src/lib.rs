use reqwest::{Client, Method};

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

pub async fn send_request(url: &str, method: Method) -> Result<HttpResponse, reqwest::Error> {
    let client = Client::new();

    let response = client.request(method, url).send().await?;

    let status = response.status().as_u16();
    let body = response.text().await?;

    Ok(HttpResponse { status, body })
}
