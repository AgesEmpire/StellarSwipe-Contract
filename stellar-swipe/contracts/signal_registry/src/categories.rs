use crate::errors::AdminError;
use soroban_sdk::{contracttype, Address, Env, Map, String, Vec};

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignalCategory {
    SwingTrade, // 1-7 days
    DayTrade,   // <24 hours
    LongTerm,   // >7 days
    Scalping,   // <1 hour
    Breakout,   // Technical breakout
    Reversal,   // Trend reversal
    Momentum,   // Momentum play
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

const MAX_TAGS: u32 = 10;
const MAX_TAG_LENGTH: u32 = 20;

pub fn validate_tag(tag: &String) -> Result<(), AdminError> {
    let bytes = tag.to_bytes();
    let len = bytes.len();

    if len == 0 || len > MAX_TAG_LENGTH {
        return Err(AdminError::InvalidParameter);
    }

    // Check alphanumeric and basic chars (letters, numbers, hyphen, underscore)
    for byte in bytes.iter() {
        let b = *byte;
        if !((b >= b'a' && b <= b'z')
            || (b >= b'A' && b <= b'Z')
            || (b >= b'0' && b <= b'9')
            || b == b'-'
            || b == b'_')
        {
            return Err(AdminError::InvalidParameter);
        }
    }

    Ok(())
}

pub fn validate_tags(tags: &Vec<String>) -> Result<(), AdminError> {
    if tags.len() > MAX_TAGS {
        return Err(AdminError::InvalidParameter);
    }

    for i in 0..tags.len() {
        validate_tag(&tags.get(i).unwrap())?;
    }

    Ok(())
}

pub fn deduplicate_tags(env: &Env, tags: Vec<String>) -> Vec<String> {
    let mut unique = Vec::new(env);

    for i in 0..tags.len() {
        let tag = tags.get(i).unwrap();
        let mut found = false;

        for j in 0..unique.len() {
            if unique.get(j).unwrap().to_bytes() == tag.to_bytes() {
                found = true;
                break;
            }
        }

        if !found {
            unique.push_back(tag);
        }
    }

    unique
}

#[contracttype]
#[derive(Clone)]
pub enum TagStorageKey {
    TagPopularity,
}

pub fn increment_tag_popularity(env: &Env, tags: &Vec<String>) {
    let mut popularity: Map<String, u32> = env
        .storage()
        .instance()
        .get(&TagStorageKey::TagPopularity)
        .unwrap_or(Map::new(env));

    for i in 0..tags.len() {
        let tag = tags.get(i).unwrap();
        let count = popularity.get(tag.clone()).unwrap_or(0);
        popularity.set(tag, count + 1);
    }

    env.storage()
        .instance()
        .set(&TagStorageKey::TagPopularity, &popularity);
}

pub fn get_popular_tags(env: &Env, limit: u32) -> Vec<(String, u32)> {
    let popularity: Map<String, u32> = env
        .storage()
        .instance()
        .get(&TagStorageKey::TagPopularity)
        .unwrap_or(Map::new(env));

    let mut tags = Vec::new(env);

    for key in popularity.keys() {
        if let Some(count) = popularity.get(key.clone()) {
            tags.push_back((key, count));
        }
    }

    // Bubble sort by count (descending)
    let len = tags.len();
    for i in 0..len {
        for j in 0..(len - i - 1) {
            let curr = tags.get(j).unwrap();
            let next = tags.get(j + 1).unwrap();

            if curr.1 < next.1 {
                let temp = curr.clone();
                tags.set(j, next);
                tags.set(j + 1, temp);
            }
        }
    }

    // Return top N
    let result_len = if limit < len { limit } else { len };
    let mut result = Vec::new(env);
    for i in 0..result_len {
        result.push_back(tags.get(i).unwrap());
    }

    result
}

pub fn auto_suggest_tags(env: &Env, rationale: &String) -> Vec<String> {
    let mut suggestions = Vec::new(env);
    let rationale_lower = rationale.to_bytes();

    // Simple keyword matching
    let keywords = [
        (b"breakout", "breakout"),
        (b"breaking", "breakout"),
        (b"resistance", "breakout"),
        (b"bullish", "bullish"),
        (b"bearish", "bearish"),
        (b"oversold", "oversold"),
        (b"overbought", "overbought"),
        (b"reversal", "reversal"),
        (b"momentum", "momentum"),
        (b"high-risk", "high-risk"),
        (b"earnings", "earnings"),
    ];

    for (keyword, tag) in keywords.iter() {
        if contains_bytes(&rationale_lower, keyword) {
            #[allow(deprecated)]
            suggestions.push_back(String::from_slice(env, tag));
            if suggestions.len() >= 5 {
                break;
            }
        }
    }

    suggestions
}

fn contains_bytes(haystack: &soroban_sdk::Bytes, needle: &[u8]) -> bool {
    let hay_len = haystack.len();
    let needle_len = needle.len();

    if needle_len > hay_len {
        return false;
    }

    for i in 0..=(hay_len - needle_len) {
        let mut matches = true;
        for j in 0..needle_len {
            if haystack.get(i + j).unwrap() != needle[j] {
                matches = false;
                break;
            }
        }
        if matches {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn test_validate_tag() {
        let env = Env::default();

        #[allow(deprecated)]
        let valid = String::from_slice(&env, "bullish");
        assert!(validate_tag(&valid).is_ok());

        #[allow(deprecated)]
        let valid_with_dash = String::from_slice(&env, "high-risk");
        assert!(validate_tag(&valid_with_dash).is_ok());

        #[allow(deprecated)]
        let too_long = String::from_slice(&env, "this_is_way_too_long_for_a_tag");
        assert!(validate_tag(&too_long).is_err());
    }

    #[test]
    fn test_deduplicate_tags() {
        let env = Env::default();
        let mut tags = Vec::new(&env);

        #[allow(deprecated)]
        {
            tags.push_back(String::from_slice(&env, "bullish"));
            tags.push_back(String::from_slice(&env, "breakout"));
            tags.push_back(String::from_slice(&env, "bullish"));
        }

        let unique = deduplicate_tags(&env, tags);
        assert_eq!(unique.len(), 2);
    }
}
