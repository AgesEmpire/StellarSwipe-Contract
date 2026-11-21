use soroban_sdk::{Env, String};

/// Errors returned by [`sanitize_string`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SanitizeError {
    /// The UTF-8 byte length of the string exceeds the allowed maximum.
    StringTooLong,
    /// The string contains an ASCII control character (byte value 0x00–0x1F).
    ControlCharacterFound,
}

/// Validate a soroban `String` field for safe use in on-chain metadata.
///
/// Checks:
/// - Byte length ≤ `max_len`.
/// - No ASCII control characters (bytes 0x00–0x1F) present.
///
/// Call sites should map the returned [`SanitizeError`] to their own
/// `ContractError` variant (e.g. `InvalidInput` or `InvalidProposal`).
pub fn sanitize_string(_env: &Env, value: &String, max_len: u32) -> Result<(), SanitizeError> {
    let bytes = value.to_bytes();
    let len = bytes.len();
    if len > max_len {
        return Err(SanitizeError::StringTooLong);
    }
    for i in 0..len {
        if bytes.get(i).unwrap() < 0x20 {
            return Err(SanitizeError::ControlCharacterFound);
        }
    }
    Ok(())
}
