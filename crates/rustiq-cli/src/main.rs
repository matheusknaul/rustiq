use reqwest::Method;
use rustiq_http::send_request;

#[tokio::main]
async fn main() {
    let url = "https://httpbin.org/get";
    let method = Method::GET;

    match send_request(url, method).await {
        Ok(response) => {
            println!("Status: {}", response.status);
            println!("Body: {}", response.body);
        }

        Err(error) => {
            eprintln!("Erro na requisição: {error}");
        }
    }
}
