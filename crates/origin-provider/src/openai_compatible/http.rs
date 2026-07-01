use std::time::Duration;

use crate::{ProviderError, ProviderResult};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

pub fn get(
    provider: &str,
    base_url: &str,
    url: &str,
    auth_header: Option<&str>,
    operation: &'static str,
) -> ProviderResult<HttpResponse> {
    request_with_timeout(
        provider,
        base_url,
        operation,
        DEFAULT_TIMEOUT,
        auth_header,
        |client| client.get(url),
    )
}

pub fn post_json_with_timeout(
    provider: &str,
    base_url: &str,
    url: &str,
    body: &str,
    auth_header: Option<&str>,
    operation: &'static str,
    timeout: Duration,
) -> ProviderResult<HttpResponse> {
    request_with_timeout(
        provider,
        base_url,
        operation,
        timeout,
        auth_header,
        |client| {
            client
                .post(url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_string())
        },
    )
}

fn request_with_timeout(
    provider: &str,
    base_url: &str,
    operation: &'static str,
    timeout: Duration,
    auth_header: Option<&str>,
    build: impl FnOnce(&reqwest::blocking::Client) -> reqwest::blocking::RequestBuilder,
) -> ProviderResult<HttpResponse> {
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|source| ProviderError::ConnectionFailed {
            provider: provider.to_string(),
            base_url: base_url.to_string(),
            operation,
            source: Box::new(source),
        })?;

    let response = apply_standard_headers(build(&client), auth_header)
        .send()
        .map_err(|source| map_reqwest_error(provider, base_url, operation, source))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .map_err(|source| map_reqwest_error(provider, base_url, operation, source))?;

    Ok(HttpResponse { status, body })
}

fn apply_standard_headers(
    builder: reqwest::blocking::RequestBuilder,
    auth_header: Option<&str>,
) -> reqwest::blocking::RequestBuilder {
    let builder = builder.header(reqwest::header::ACCEPT, "application/json");
    if let Some(auth_header) = auth_header {
        builder.header(reqwest::header::AUTHORIZATION, auth_header)
    } else {
        builder
    }
}

fn map_reqwest_error(
    provider: &str,
    base_url: &str,
    operation: &'static str,
    source: reqwest::Error,
) -> ProviderError {
    if source.is_timeout() {
        ProviderError::Timeout {
            provider: provider.to_string(),
            base_url: base_url.to_string(),
            operation,
            source: Box::new(source),
        }
    } else if source.is_builder() || source.is_request() || source.is_connect() {
        ProviderError::ConnectionFailed {
            provider: provider.to_string(),
            base_url: base_url.to_string(),
            operation,
            source: Box::new(source),
        }
    } else {
        ProviderError::InvalidResponse {
            provider: provider.to_string(),
            message: source.to_string(),
            source: Some(Box::new(source)),
        }
    }
}

#[cfg(test)]
mod tests {
    use reqwest::header::{ACCEPT, AUTHORIZATION};

    use super::apply_standard_headers;

    #[test]
    fn applies_accept_header_without_auth() {
        let client = reqwest::blocking::Client::new();
        let request = apply_standard_headers(client.get("https://example.test"), None)
            .build()
            .unwrap();

        assert_eq!(request.headers().get(ACCEPT).unwrap(), "application/json");
        assert!(request.headers().get(AUTHORIZATION).is_none());
    }

    #[test]
    fn applies_authorization_header_when_present() {
        let client = reqwest::blocking::Client::new();
        let request =
            apply_standard_headers(client.get("https://example.test"), Some("Bearer test-key"))
                .build()
                .unwrap();

        assert_eq!(
            request.headers().get(AUTHORIZATION).unwrap(),
            "Bearer test-key"
        );
    }
}
