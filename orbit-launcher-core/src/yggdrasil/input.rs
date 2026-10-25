//! authlib-injector's encoded drag payload and ordinary endpoint input share
//! the same URL/security validation. Decode only the protocol component, once.

use crate::error::LauncherError;

const DROP_PREFIX: &str = "authlib-injector:yggdrasil-server:";
const MAX_ADDRESS_BYTES: usize = 16 * 1024;

pub(super) fn parse_entered_address(
    address: &str,
    allow_insecure_http: bool,
) -> Result<url::Url, LauncherError> {
    let address = address.trim();
    if address.is_empty()
        || address.len() > MAX_ADDRESS_BYTES
        || address.chars().any(char::is_control)
    {
        return Err(LauncherError::InvalidConfig(
            "Yggdrasil endpoint address is empty, too long, or contains control characters".into(),
        ));
    }
    let decoded;
    let address = if let Some(encoded) = address.strip_prefix(DROP_PREFIX) {
        let bytes = encoded.as_bytes();
        for (index, byte) in bytes.iter().enumerate() {
            if *byte == b'%'
                && !(bytes.get(index + 1).is_some_and(u8::is_ascii_hexdigit)
                    && bytes.get(index + 2).is_some_and(u8::is_ascii_hexdigit))
            {
                return Err(LauncherError::InvalidConfig(
                    "authlib-injector drag URI has invalid percent encoding".into(),
                ));
            }
        }
        decoded = percent_encoding::percent_decode_str(encoded)
            .decode_utf8()
            .map_err(|_| {
                LauncherError::InvalidConfig("authlib-injector drag URI is not valid UTF-8".into())
            })?;
        if decoded.is_empty() || decoded.chars().any(char::is_control) {
            return Err(LauncherError::InvalidConfig(
                "authlib-injector drag URI has an empty or invalid endpoint".into(),
            ));
        }
        decoded.as_ref()
    } else if address.starts_with("authlib-injector:") {
        return Err(LauncherError::InvalidConfig(
            "unsupported authlib-injector configuration URI".into(),
        ));
    } else {
        address
    };
    let parsed = match url::Url::parse(address) {
        Ok(url) if url.has_host() || matches!(url.scheme(), "http" | "https") => url,
        Ok(url) => {
            // host:port shorthand resembles an opaque URI to the URL parser.
            let numeric_port = address.split_once(':').is_some_and(|(_, tail)| {
                let port = tail.split('/').next().unwrap_or("");
                !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit())
            });
            if !numeric_port {
                return Err(LauncherError::InvalidConfig(format!(
                    "unsupported Yggdrasil endpoint scheme '{}'",
                    url.scheme()
                )));
            }
            url::Url::parse(&format!("https://{address}")).map_err(|error| {
                LauncherError::InvalidConfig(format!("invalid Yggdrasil endpoint address: {error}"))
            })?
        }
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            url::Url::parse(&format!("https://{address}")).map_err(|error| {
                LauncherError::InvalidConfig(format!("invalid Yggdrasil endpoint address: {error}"))
            })?
        }
        Err(error) => {
            return Err(LauncherError::InvalidConfig(format!(
                "invalid Yggdrasil endpoint address: {error}"
            )));
        }
    };
    super::normalized_api_url(parsed, allow_insecure_http)
        .map_err(|error| LauncherError::InvalidConfig(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_component_is_decoded_once_without_form_plus_conversion() {
        let result = parse_entered_address(
            "authlib-injector:yggdrasil-server:https%3A%2F%2Fskin.example%2Fa%2Bb%2520c",
            false,
        )
        .unwrap();
        assert_eq!(result.as_str(), "https://skin.example/a+b%20c/");
        assert_eq!(
            parse_entered_address("https://skin.example/a%2Fb", false)
                .unwrap()
                .as_str(),
            "https://skin.example/a%2Fb/"
        );
    }

    #[test]
    fn encoded_input_keeps_all_endpoint_security_checks() {
        for payload in [
            "",
            "%",
            "%2",
            "%GG",
            "%ff",
            "https%3A%2F%2Fskin.example%0Aevil",
            "file%3A%2F%2F%2Ftmp",
            "https%3A%2F%2Fuser%3Apassword%40skin.example",
            "https%3A%2F%2Fskin.example%2F%3Fx%3D1",
            "https%3A%2F%2Fskin.example%23fragment",
        ] {
            assert!(
                parse_entered_address(&format!("{DROP_PREFIX}{payload}"), false).is_err(),
                "{payload}"
            );
        }
        assert!(
            parse_entered_address("authlib-injector:unknown:https%3A%2F%2Fskin.example", false)
                .is_err()
        );
        assert!(parse_entered_address(&"a".repeat(MAX_ADDRESS_BYTES + 1), false).is_err());
    }

    #[test]
    fn encoded_http_still_needs_explicit_permission() {
        let input = "authlib-injector:yggdrasil-server:http%3A%2F%2Fskin.example%2Fapi";
        assert!(parse_entered_address(input, false).is_err());
        assert_eq!(
            parse_entered_address(input, true).unwrap().as_str(),
            "http://skin.example/api/"
        );
    }
}
