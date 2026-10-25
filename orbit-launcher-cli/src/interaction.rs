//! Machine interaction uses the same install decision as the terminal prompt.
use std::io::{BufRead, Write};

use orbit_i18n::tr;
use orbit_launcher_core::{EulaDocument, LauncherError};
use orbit_machine_protocol::{
    InteractionChoice, InteractionEnvelope, InteractionKind, InteractionResponse, SCHEMA_VERSION,
};

pub fn confirm_eula(
    document: &EulaDocument,
    sequence: u64,
    input: impl BufRead,
    mut output: impl Write,
) -> Result<bool, LauncherError> {
    let id = format!("eula-{sequence}-{}", document.digest_sha256);
    let envelope = InteractionEnvelope::new(
        "install",
        sequence,
        &id,
        InteractionKind::Confirmation,
        tr!("Review the Minecraft EULA before installing").into_owned(),
        vec![
            InteractionChoice {
                id: "accept".into(),
                label: tr!("I agree").into_owned(),
                description: None,
                data: serde_json::json!({"document": {
                    "text": document.text, "url": document.url,
                    "digest_sha256": document.digest_sha256,
                }}),
            },
            InteractionChoice {
                id: "decline".into(),
                label: tr!("Cancel").into_owned(),
                description: None,
                data: serde_json::json!({}),
            },
        ],
    )
    .with_default("decline");
    serde_json::to_writer(&mut output, &envelope)
        .map_err(|error| LauncherError::InteractionRequired(error.to_string()))?;
    writeln!(output)?;
    output.flush()?;
    read_acceptance(input, &id)
}

fn read_acceptance(input: impl BufRead, id: &str) -> Result<bool, LauncherError> {
    // Responses contain only IDs, never documents or credentials.
    let mut bounded = std::io::Read::take(input, 16 * 1024);
    let mut line = String::new();
    if bounded.read_line(&mut line)? == 0 {
        return Ok(false);
    }
    let response: InteractionResponse = serde_json::from_str(&line)
        .map_err(|error| LauncherError::InteractionRequired(error.to_string()))?;
    if response.schema_version != SCHEMA_VERSION
        || response.kind != "interaction_response"
        || response.interaction_id != id
    {
        return Err(LauncherError::InteractionRequired(
            "Invalid EULA interaction response".into(),
        ));
    }
    if response.cancelled {
        return Ok(false);
    }
    match response.selected_choice.as_deref() {
        Some("accept") => Ok(true),
        Some("decline") => Ok(false),
        _ => Err(LauncherError::InteractionRequired(
            "Unknown EULA response".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn response(id: &str, choice: &str) -> Vec<u8> {
        serde_json::to_vec(&InteractionResponse::selected(id, choice)).unwrap()
    }
    #[test]
    fn emits_the_complete_document_with_a_safe_default() {
        let document = EulaDocument {
            url: "https://example.invalid/eula".into(),
            digest_sha256: "digest".into(),
            fetched_at_unix_seconds: 0,
            text: "First paragraph.\n\nSecond paragraph.\n".into(),
        };
        let mut output = Vec::new();
        assert!(
            confirm_eula(
                &document,
                3,
                response("eula-3-digest", "accept").as_slice(),
                &mut output
            )
            .unwrap()
        );
        let envelope: InteractionEnvelope<serde_json::Value> =
            serde_json::from_slice(&output).unwrap();
        assert_eq!(envelope.default_choice.as_deref(), Some("decline"));
        assert_eq!(envelope.choices[0].data["document"]["text"], document.text);
        assert_eq!(
            envelope.choices[0].data["document"]["digest_sha256"],
            document.digest_sha256
        );
    }
    #[test]
    fn only_explicit_acceptance_of_the_current_document_is_accepted() {
        assert!(read_acceptance(response("current", "accept").as_slice(), "current").unwrap());
        assert!(!read_acceptance(response("current", "decline").as_slice(), "current").unwrap());
        assert!(read_acceptance(response("old", "accept").as_slice(), "current").is_err());
        assert!(read_acceptance(response("current", "yes").as_slice(), "current").is_err());
        assert!(!read_acceptance(&b""[..], "current").unwrap());
    }
}
