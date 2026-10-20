use soroban_sdk::{contracttype, Address, Env, Map, String, Vec};

use crate::types::SignalAction;

pub const MAX_TEMPLATES_PER_PROVIDER: u32 = 5;

#[contracttype]
#[derive(Clone, Debug)]
pub struct SignalTemplatePreset {
    pub asset_pair: String,
    pub action: SignalAction,
    pub risk_rating: u32,
    pub category: String,
    pub default_expiry_hours: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SignalTemplateOverrides {
    pub asset_pair: Option<String>,
    pub action: Option<u32>,
    pub risk_rating: Option<u32>,
    pub category: Option<String>,
    pub expiry_hours: Option<u64>,
    pub price: Option<i128>,
    pub rationale: Option<String>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct StoredSignalTemplate {
    pub template_id: u32,
    pub template: SignalTemplatePreset,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PresetTemplateError {
    TemplateLimitReached,
    TemplateNotFound,
}

pub fn save_signal_template(
    env: &Env,
    templates: &mut Map<Address, Vec<StoredSignalTemplate>>,
    provider: Address,
    template: SignalTemplatePreset,
) -> Result<u32, PresetTemplateError> {
    let mut provider_templates = templates.get(provider.clone()).unwrap_or(Vec::new(env));
    if provider_templates.len() >= MAX_TEMPLATES_PER_PROVIDER {
        return Err(PresetTemplateError::TemplateLimitReached);
    }

    let template_id = provider_templates.len() + 1;
    provider_templates.push_back(StoredSignalTemplate {
        template_id,
        template,
    });
    templates.set(provider, provider_templates);

    Ok(template_id)
}

pub fn get_signal_template(
    templates: &Map<Address, Vec<StoredSignalTemplate>>,
    provider: Address,
    template_id: u32,
) -> Result<SignalTemplatePreset, PresetTemplateError> {
    let provider_templates = templates
        .get(provider)
        .ok_or(PresetTemplateError::TemplateNotFound)?;

    for i in 0..provider_templates.len() {
        if let Some(stored) = provider_templates.get(i) {
            if stored.template_id == template_id {
                return Ok(stored.template);
            }
        }
    }

    Err(PresetTemplateError::TemplateNotFound)
}

pub fn merge_template(
    template: SignalTemplatePreset,
    overrides: SignalTemplateOverrides,
) -> (String, SignalAction, u64, i128, String) {
    let asset_pair = overrides.asset_pair.unwrap_or(template.asset_pair);
    let action = match overrides.action {
        Some(1) => SignalAction::Sell,
        Some(0) => SignalAction::Buy,
        _ => template.action,
    };
    let expiry_hours = overrides
        .expiry_hours
        .unwrap_or(template.default_expiry_hours);
    let price = overrides.price.unwrap_or(1);
    let rationale = overrides
        .rationale
        .unwrap_or(overrides.category.unwrap_or(template.category));

    (asset_pair, action, expiry_hours, price, rationale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    fn sdk_string(env: &Env, value: &str) -> String {
        String::from_str(env, value)
    }

    fn template(env: &Env) -> SignalTemplatePreset {
        SignalTemplatePreset {
            asset_pair: sdk_string(env, "XLM/USDC"),
            action: SignalAction::Buy,
            risk_rating: 2,
            category: sdk_string(env, "momentum"),
            default_expiry_hours: 24,
        }
    }

    #[test]
    fn save_template() {
        let env = Env::default();
        let provider = Address::generate(&env);
        let mut templates = Map::new(&env);

        let template_id =
            save_signal_template(&env, &mut templates, provider.clone(), template(&env)).unwrap();

        assert_eq!(template_id, 1);
        assert_eq!(templates.get(provider).unwrap().len(), 1);
    }

    #[test]
    fn merge_template_values_without_overrides() {
        let env = Env::default();
        let (asset_pair, action, expiry_hours, price, rationale) = merge_template(
            template(&env),
            SignalTemplateOverrides {
                asset_pair: None,
                action: None,
                risk_rating: None,
                category: None,
                expiry_hours: None,
                price: None,
                rationale: None,
            },
        );

        assert_eq!(asset_pair, sdk_string(&env, "XLM/USDC"));
        assert!(matches!(action, SignalAction::Buy));
        assert_eq!(expiry_hours, 24);
        assert_eq!(price, 1);
        assert_eq!(rationale, sdk_string(&env, "momentum"));
    }

    #[test]
    fn override_fields() {
        let env = Env::default();
        let (asset_pair, action, expiry_hours, price, rationale) = merge_template(
            template(&env),
            SignalTemplateOverrides {
                asset_pair: Some(sdk_string(&env, "BTC/USDC")),
                action: Some(1),
                risk_rating: Some(5),
                category: Some(sdk_string(&env, "hedge")),
                expiry_hours: Some(12),
                price: Some(50),
                rationale: Some(sdk_string(&env, "override")),
            },
        );

        assert_eq!(asset_pair, sdk_string(&env, "BTC/USDC"));
        assert!(matches!(action, SignalAction::Sell));
        assert_eq!(expiry_hours, 12);
        assert_eq!(price, 50);
        assert_eq!(rationale, sdk_string(&env, "override"));
    }

    #[test]
    fn template_limit() {
        let env = Env::default();
        let provider = Address::generate(&env);
        let mut templates = Map::new(&env);

        for _ in 0..MAX_TEMPLATES_PER_PROVIDER {
            save_signal_template(&env, &mut templates, provider.clone(), template(&env)).unwrap();
        }

        let result = save_signal_template(&env, &mut templates, provider, template(&env));
        assert_eq!(result, Err(PresetTemplateError::TemplateLimitReached));
    }
}
