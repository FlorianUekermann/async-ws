use http::request::Builder;
use http::{HeaderValue, Method, Request, Response, StatusCode};
use rand::{thread_rng, Rng};
use ring::digest::{Context, SHA1_FOR_LEGACY_USE_ONLY};

pub fn upgrade_request() -> Builder {
    let mut nonce = [0u8; 16];
    thread_rng().fill(&mut nonce);
    Request::builder()
        .method(Method::GET)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", base64::encode(nonce))
}

pub fn is_upgrade_request<T>(request: &Request<T>) -> bool {
    request.method() == http::Method::GET
        && request
            .headers()
            .get("Connection")
            .iter()
            .flat_map(|v| v.as_bytes().split(|&c| c == b' ' || c == b','))
            .filter(|h| h.eq_ignore_ascii_case(b"Upgrade"))
            .next()
            .is_some()
        && request
            .headers()
            .get("Upgrade")
            .filter(|v| v.as_bytes().eq_ignore_ascii_case(b"websocket"))
            .is_some()
        && request
            .headers()
            .get("Sec-WebSocket-Version")
            .map(HeaderValue::as_bytes)
            == Some(b"13")
        && request.headers().get("Sec-WebSocket-Key").is_some()
}

pub fn upgrade_response<T>(request: &Request<T>) -> Option<Response<()>> {
    let challenge = match (
        is_upgrade_request(request),
        request.headers().get("Sec-WebSocket-Key"),
    ) {
        (false, _) | (true, None) => return None,
        (true, Some(challenge)) => challenge.as_bytes(),
    };

    let response = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .version(request.version())
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header(
            "Sec-WebSocket-Accept",
            upgrade_challenge_response(challenge),
        )
        .body(())
        .unwrap();
    Some(response)
}

fn upgrade_challenge_response(challenge: &[u8]) -> String {
    let mut ctx = Context::new(&SHA1_FOR_LEGACY_USE_ONLY);
    ctx.update(challenge);
    ctx.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64::encode(ctx.finish())
}

#[cfg(test)]
mod tests {
    use crate::http::upgrade_challenge_response;

    #[test]
    fn challenge_response() {
        assert_eq!(
            upgrade_challenge_response(b"dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }
}
