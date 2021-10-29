use anyhow::bail;
use http::{HeaderValue, Request, Response, StatusCode};
use ring::digest::{Context, SHA1_FOR_LEGACY_USE_ONLY};

pub fn is_upgrade_request(request: &Request<()>) -> bool {
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

pub fn upgrade_response(request: &Request<()>) -> anyhow::Result<Response<()>> {
    let challenge = match (
        is_upgrade_request(request),
        request.headers().get("Sec-WebSocket-Key"),
    ) {
        (false, _) | (true, None) => bail!("not an upgrade request"),
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
        .body(())?;
    Ok(response)
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
