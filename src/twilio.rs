use base64::{Engine as _, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

pub(crate) type FormFields = Vec<(String, String)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComplianceKeyword {
    Stop,
    Start,
    Help,
}

/// Decode an `application/x-www-form-urlencoded` request body.
///
/// A vector is intentional: Twilio signs every submitted pair, including
/// duplicate field names, and may add fields to webhook requests over time.
pub(crate) fn parse_form_body(body: &[u8]) -> FormFields {
    url::form_urlencoded::parse(body).into_owned().collect()
}

pub(crate) fn form_field<'a>(fields: &'a [(String, String)], name: &str) -> Option<&'a str> {
    fields
        .iter()
        .find_map(|(field, value)| (field == name).then_some(value.as_str()))
}

/// Compute a Twilio webhook signature for an exact public request URL.
///
/// The caller must supply the externally visible URL, including its query
/// string. Reconstructing it from an internal request behind a proxy can
/// produce a different signature input.
#[allow(dead_code)]
pub(crate) fn twilio_signature(
    auth_token: &str,
    public_url: &str,
    fields: &[(String, String)],
) -> String {
    STANDARD.encode(
        signed_mac(auth_token, public_url, fields)
            .finalize()
            .into_bytes(),
    )
}

/// Validate a Twilio webhook signature using the MAC implementation's
/// constant-time tag comparison.
pub(crate) fn verify_twilio_signature(
    auth_token: &str,
    public_url: &str,
    fields: &[(String, String)],
    supplied_signature: &str,
) -> bool {
    let Ok(signature) = STANDARD.decode(supplied_signature.trim()) else {
        return false;
    };

    signed_mac(auth_token, public_url, fields)
        .verify_slice(&signature)
        .is_ok()
}

fn signed_mac(auth_token: &str, public_url: &str, fields: &[(String, String)]) -> HmacSha1 {
    let mut sorted_fields: Vec<_> = fields.iter().collect();

    // `sort_by` is stable. The original order is therefore retained when a
    // webhook contains the same parameter name more than once.
    sorted_fields.sort_by(|(left, _), (right, _)| left.cmp(right));

    let mut mac = HmacSha1::new_from_slice(auth_token.as_bytes())
        .expect("HMAC accepts authentication tokens of any length");
    mac.update(public_url.as_bytes());
    for (name, value) in sorted_fields {
        mac.update(name.as_bytes());
        mac.update(value.as_bytes());
    }
    mac
}

/// Recognize exact carrier-compliance commands without treating ordinary
/// sentences such as "help me with Rust" as commands.
pub(crate) fn compliance_keyword(body: &str) -> Option<ComplianceKeyword> {
    match body.trim().to_ascii_uppercase().as_str() {
        "STOP" | "STOPALL" | "UNSUBSCRIBE" | "CANCEL" | "END" | "QUIT" | "REVOKE" | "OPTOUT" => {
            Some(ComplianceKeyword::Stop)
        }
        "START" | "UNSTOP" => Some(ComplianceKeyword::Start),
        "HELP" | "INFO" => Some(ComplianceKeyword::Help),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_twilio_official_signature_test_vector() {
        let fields = vec![
            ("CallSid".to_owned(), "CA1234567890ABCDE".to_owned()),
            ("Caller".to_owned(), "+14158675310".to_owned()),
            ("Digits".to_owned(), "1234".to_owned()),
            ("From".to_owned(), "+14158675310".to_owned()),
            ("To".to_owned(), "+18005551212".to_owned()),
        ];
        let url = "https://example.com/myapp.php?foo=1&bar=2";
        let expected = "L/OH5YylLD5NRKLltdqwSvS0BnU=";

        assert_eq!(twilio_signature("12345", url, &fields), expected);
        assert!(verify_twilio_signature("12345", url, &fields, expected));
        assert!(!verify_twilio_signature(
            "wrong-token",
            url,
            &fields,
            expected
        ));
    }

    #[test]
    fn parses_encoded_values_and_preserves_duplicate_pairs() {
        let fields = parse_form_body(
            b"Body=I+clone+everything%21&MediaUrl=https%3A%2F%2Fexample.test%2Fa&MediaUrl=https%3A%2F%2Fexample.test%2Fb",
        );

        assert_eq!(form_field(&fields, "Body"), Some("I clone everything!"));
        assert_eq!(
            fields,
            vec![
                ("Body".to_owned(), "I clone everything!".to_owned()),
                ("MediaUrl".to_owned(), "https://example.test/a".to_owned()),
                ("MediaUrl".to_owned(), "https://example.test/b".to_owned()),
            ]
        );
    }

    #[test]
    fn signature_is_independent_of_distinct_parameter_input_order() {
        let first = vec![
            ("To".to_owned(), "+18005551212".to_owned()),
            ("Body".to_owned(), "hello".to_owned()),
        ];
        let second = vec![first[1].clone(), first[0].clone()];

        assert_eq!(
            twilio_signature("token", "https://example.test/hook", &first),
            twilio_signature("token", "https://example.test/hook", &second)
        );
    }

    #[test]
    fn recognizes_only_exact_compliance_keywords() {
        assert_eq!(
            compliance_keyword(" stop \n"),
            Some(ComplianceKeyword::Stop)
        );
        assert_eq!(compliance_keyword("UnStop"), Some(ComplianceKeyword::Start));
        assert_eq!(compliance_keyword("info"), Some(ComplianceKeyword::Help));
        assert_eq!(compliance_keyword("help me with Rust"), None);
        assert_eq!(compliance_keyword("unstoppable"), None);
    }

    #[test]
    fn rejects_malformed_base64_signature() {
        assert!(!verify_twilio_signature(
            "token",
            "https://example.test/hook",
            &Vec::new(),
            "not base64"
        ));
    }
}
