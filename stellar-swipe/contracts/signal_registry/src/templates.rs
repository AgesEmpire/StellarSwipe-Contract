extern crate alloc;

use alloc::string::{String as RustString, ToString};
use alloc::vec::Vec as RustVec;
use core::str;
use soroban_sdk::{contracttype, Address, Env, Map, String};

use crate::errors::TemplateError;
use crate::StorageKey;

pub const DEFAULT_TEMPLATE_EXPIRY_HOURS: u32 = 24;
pub const MAX_SIGNAL_RATIONALE_BYTES: u32 = 500;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignalTemplate {
    pub id: u64,
    pub provider: Address,
    pub name: String,
    pub asset_pair: Option<String>,
    pub action: Option<String>,
    pub rationale_template: String,
    pub default_expiry_hours: u32,
    pub is_public: bool,
    pub use_count: u32,
}

pub fn get_next_template_id(env: &Env) -> u64 {
    let mut counter: u64 = env
        .storage()
        .instance()
        .get(&StorageKey::TemplateCounter)
        .unwrap_or(0);
    counter = counter.checked_add(1).expect("template id overflow");
    env.storage()
        .instance()
        .set(&StorageKey::TemplateCounter, &counter);
    counter
}

pub fn get_templates_map(env: &Env) -> Map<u64, SignalTemplate> {
    env.storage()
        .instance()
        .get(&StorageKey::Templates)
        .unwrap_or(Map::new(env))
}

pub fn store_template(env: &Env, template_id: u64, template: &SignalTemplate) {
    let mut templates = get_templates_map(env);
    templates.set(template_id, template.clone());
    env.storage()
        .instance()
        .set(&StorageKey::Templates, &templates);
}

pub fn get_template(env: &Env, template_id: u64) -> Option<SignalTemplate> {
    let templates = get_templates_map(env);
    templates.get(template_id)
}

pub fn increment_template_use_count(env: &Env, template_id: u64) -> Result<(), TemplateError> {
    let mut templates = get_templates_map(env);
    let mut template = templates
        .get(template_id)
        .ok_or(TemplateError::TemplateNotFound)?;
    template.use_count = template
        .use_count
        .checked_add(1)
        .ok_or(TemplateError::InvalidTemplate)?;
    templates.set(template_id, template);
    env.storage()
        .instance()
        .set(&StorageKey::Templates, &templates);
    Ok(())
}

pub fn set_template_visibility(
    env: &Env,
    provider: &Address,
    template_id: u64,
    is_public: bool,
) -> Result<(), TemplateError> {
    let mut templates = get_templates_map(env);
    let mut template = templates
        .get(template_id)
        .ok_or(TemplateError::TemplateNotFound)?;
    if &template.provider != provider {
        return Err(TemplateError::Unauthorized);
    }
    template.is_public = is_public;
    templates.set(template_id, template);
    env.storage()
        .instance()
        .set(&StorageKey::Templates, &templates);
    Ok(())
}

pub fn replace_variables(
    env: &Env,
    template: &String,
    variables: &Map<String, String>,
) -> Result<String, TemplateError> {
    let template_text = soroban_to_rust_string(template)?;
    let mut out = RustString::new();

    let bytes = template_text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'}' {
                j += 1;
            }

            if j == bytes.len() {
                out.push('{');
                i += 1;
                continue;
            }

            let key =
                str::from_utf8(&bytes[(i + 1)..j]).map_err(|_| TemplateError::InvalidTemplate)?;
            if key.is_empty() {
                return Err(TemplateError::InvalidTemplate);
            }

            if let Some(value) = get_variable(variables, key)? {
                out.push_str(soroban_to_rust_string(&value)?.as_str());
            } else if key == "date" {
                out.push_str(env.ledger().timestamp().to_string().as_str());
            } else {
                return Err(TemplateError::MissingVariable);
            }

            i = j + 1;
            continue;
        }

        out.push(bytes[i] as char);
        i += 1;
    }

    let mut out_bytes = out.into_bytes();
    if out_bytes.len() > MAX_SIGNAL_RATIONALE_BYTES as usize {
        out_bytes.truncate(MAX_SIGNAL_RATIONALE_BYTES as usize);
    }

    let out_text = str::from_utf8(&out_bytes).map_err(|_| TemplateError::InvalidTemplate)?;
    Ok(String::from_str(env, out_text))
}

pub fn get_variable(
    variables: &Map<String, String>,
    key: &str,
) -> Result<Option<String>, TemplateError> {
    for map_key in variables.keys() {
        let key_text = soroban_to_rust_string(&map_key)?;
        if key_text == key {
            return Ok(variables.get(map_key));
        }
    }
    Ok(None)
}

pub fn parse_action(action_text: &String) -> Result<crate::types::SignalAction, TemplateError> {
    let action = soroban_to_rust_string(action_text)?;
    let lower = action.to_ascii_lowercase();
    match lower.as_str() {
        "buy" => Ok(crate::types::SignalAction::Buy),
        "sell" => Ok(crate::types::SignalAction::Sell),
        _ => Err(TemplateError::InvalidAction),
    }
}

pub fn parse_price(price_text: &String) -> Result<i128, TemplateError> {
    let price_str = soroban_to_rust_string(price_text)?;
    let price = price_str
        .parse::<i128>()
        .map_err(|_| TemplateError::MissingVariable)?;
    if price <= 0 {
        return Err(TemplateError::MissingVariable);
    }
    Ok(price)
}

fn soroban_to_rust_string(value: &String) -> Result<RustString, TemplateError> {
    let bytes = value.clone().to_bytes();
    let mut raw = RustVec::with_capacity(bytes.len() as usize);
    for i in 0..bytes.len() {
        raw.push(bytes.get(i).unwrap());
    }
    let text = str::from_utf8(&raw).map_err(|_| TemplateError::InvalidTemplate)?;
    Ok(text.to_string())
}
