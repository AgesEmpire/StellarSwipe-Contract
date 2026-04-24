use crate::errors::ImportError;
use crate::types::SignalAction;
use soroban_sdk::{Address, Bytes, Env, Map, String, Vec};

const MAX_BATCH_SIZE: u32 = 100;
const MAX_RATIONALE_LEN: u32 = 500;

pub struct ImportResult {
    pub success_count: u32,
    pub error_count: u32,
}

pub fn import_signals_csv(
    env: &Env,
    _provider: &Address,
    data: Bytes,
    validate_only: bool,
) -> ImportResult {
    let mut success_count = 0;
    let mut error_count = 0;

    // Convert bytes to string for parsing
    let data_vec = bytes_to_vec(&data);
    let lines = parse_csv_lines(&data_vec);

    if lines.is_empty() {
        return ImportResult {
            success_count: 0,
            error_count: 1,
        };
    }

    // Skip header (first line)
    for i in 1..lines.len().min(MAX_BATCH_SIZE as usize + 1) {
        if success_count >= MAX_BATCH_SIZE {
            error_count += 1;
            break;
        }

        match validate_csv_line(&lines[i]) {
            Ok(_) => {
                if !validate_only {
                    success_count += 1;
                }
            }
            Err(_) => error_count += 1,
        }
    }

    ImportResult {
        success_count,
        error_count,
    }
}

pub fn import_signals_json(
    _env: &Env,
    _provider: &Address,
    _data: Bytes,
    _validate_only: bool,
) -> ImportResult {
    // JSON import placeholder
    ImportResult {
        success_count: 0,
        error_count: 0,
    }
}

fn bytes_to_vec(bytes: &Bytes) -> alloc::vec::Vec<u8> {
    let mut vec = alloc::vec::Vec::new();
    for i in 0..bytes.len() {
        vec.push(bytes.get(i).unwrap());
    }
    vec
}

fn parse_csv_lines(data: &[u8]) -> alloc::vec::Vec<alloc::vec::Vec<alloc::vec::Vec<u8>>> {
    let mut lines = alloc::vec::Vec::new();
    let mut current_line = alloc::vec::Vec::new();
    let mut current_field = alloc::vec::Vec::new();

    for &byte in data {
        if byte == b'\n' || byte == b'\r' {
            if !current_field.is_empty() || !current_line.is_empty() {
                current_line.push(current_field.clone());
                current_field.clear();
            }
            if !current_line.is_empty() {
                lines.push(current_line.clone());
                current_line.clear();
            }
        } else if byte == b',' {
            current_line.push(current_field.clone());
            current_field.clear();
        } else {
            current_field.push(byte);
        }
    }

    if !current_field.is_empty() {
        current_line.push(current_field);
    }
    if !current_line.is_empty() {
        lines.push(current_line);
    }

    lines
}

fn validate_csv_line(fields: &[alloc::vec::Vec<u8>]) -> Result<(), ImportError> {
    if fields.len() < 5 {
        return Err(ImportError::InvalidFormat);
    }

    // Validate asset pair (must contain '/')
    if !contains_byte(&fields[0], b'/') {
        return Err(ImportError::InvalidAssetPair);
    }

    // Validate action
    validate_action(&fields[1])?;

    // Validate price
    let price = parse_i128_from_bytes(&fields[2])?;
    if price <= 0 {
        return Err(ImportError::InvalidPrice);
    }

    // Validate rationale
    if fields[3].is_empty() || fields[3].len() > MAX_RATIONALE_LEN as usize {
        return Err(ImportError::InvalidRationale);
    }

    // Validate expiry
    let expiry = parse_u32_from_bytes(&fields[4])?;
    if expiry == 0 || expiry > 720 {
        return Err(ImportError::InvalidExpiry);
    }

    Ok(())
}

fn contains_byte(data: &[u8], byte: u8) -> bool {
    data.iter().any(|&b| b == byte)
}

fn validate_action(data: &[u8]) -> Result<SignalAction, ImportError> {
    let trimmed = trim_bytes(data);

    if trimmed.len() == 3 {
        let upper = [
            to_upper(trimmed[0]),
            to_upper(trimmed[1]),
            to_upper(trimmed[2]),
        ];
        if &upper == b"BUY" {
            return Ok(SignalAction::Buy);
        }
    }

    if trimmed.len() == 4 {
        let upper = [
            to_upper(trimmed[0]),
            to_upper(trimmed[1]),
            to_upper(trimmed[2]),
            to_upper(trimmed[3]),
        ];
        if &upper == b"SELL" {
            return Ok(SignalAction::Sell);
        }
    }

    Err(ImportError::InvalidAction)
}

fn parse_i128_from_bytes(data: &[u8]) -> Result<i128, ImportError> {
    let trimmed = trim_bytes(data);
    if trimmed.is_empty() {
        return Err(ImportError::InvalidPrice);
    }

    let mut result: i128 = 0;
    let mut negative = false;
    let mut started = false;

    for &byte in trimmed {
        if byte == b'-' && !started {
            negative = true;
            started = true;
        } else if byte >= b'0' && byte <= b'9' {
            result = result * 10 + (byte - b'0') as i128;
            started = true;
        } else if byte == b'.' {
            break; // Ignore decimal part
        } else if byte != b' ' {
            return Err(ImportError::InvalidPrice);
        }
    }

    if !started {
        return Err(ImportError::InvalidPrice);
    }

    Ok(if negative { -result } else { result })
}

fn parse_u32_from_bytes(data: &[u8]) -> Result<u32, ImportError> {
    let value = parse_i128_from_bytes(data)?;
    if value < 0 || value > u32::MAX as i128 {
        return Err(ImportError::InvalidFormat);
    }
    Ok(value as u32)
}

fn trim_bytes(data: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = data.len();

    while start < end && (data[start] == b' ' || data[start] == b'\t') {
        start += 1;
    }

    while end > start && (data[end - 1] == b' ' || data[end - 1] == b'\t') {
        end -= 1;
    }

    &data[start..end]
}

fn to_upper(byte: u8) -> u8 {
    if byte >= b'a' && byte <= b'z' {
        byte - 32
    } else {
        byte
    }
}

// External ID mapping
pub fn store_external_id_mapping(
    env: &Env,
    provider: &Address,
    external_id: &String,
    signal_id: u64,
) {
    let key = (provider.clone(), external_id.clone());
    let mut mappings: Map<(Address, String), u64> = env
        .storage()
        .persistent()
        .get(&crate::StorageKey::ExternalIdMappings)
        .unwrap_or(Map::new(env));

    mappings.set(key, signal_id);
    env.storage()
        .persistent()
        .set(&crate::StorageKey::ExternalIdMappings, &mappings);
}

pub fn get_signal_by_external_id(
    env: &Env,
    provider: &Address,
    external_id: &String,
) -> Option<u64> {
    let mappings: Map<(Address, String), u64> = env
        .storage()
        .persistent()
        .get(&crate::StorageKey::ExternalIdMappings)?;

    mappings.get((provider.clone(), external_id.clone()))
}

extern crate alloc;
