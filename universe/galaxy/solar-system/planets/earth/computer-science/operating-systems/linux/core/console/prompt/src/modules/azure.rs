use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::{Context, Module, ModuleConfig};

use crate::configs::azure::AzureConfig;
use crate::formatter::StringFormatter;
use crate::utils::read_file;

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AzureProfile {
    installation_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    subscriptions: Vec<Subscription>,
}

#[derive(Serialize, Deserialize, Clone)]
struct User {
    name: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Subscription {
    name: String,
    user: User,
    is_default: bool,
}

pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("azure");
    let config = AzureConfig::try_load(module.config);

    if config.disabled {
        return None;
    }

    let subscription: Option<Subscription> = get_azure_profile_info(context);

    if subscription.is_none() {
        log::info!("Could not find Subscriptions in azureProfile.json");
        return None;
    }

    let subscription = subscription.unwrap();

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|variable, _| match variable {
                "symbol" => Some(config.symbol),
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "subscription" => Some(Ok(config
                    .subscription_aliases
                    .get(&subscription.name)
                    .copied()
                    .unwrap_or(&subscription.name))),
                "username" => Some(Ok(&subscription.user.name)),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `azure`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn get_azure_profile_info(context: &Context) -> Option<Subscription> {
    let mut config_path = get_config_file_location(context)?;
    config_path.push("azureProfile.json");

    let azure_profile = load_azure_profile(&config_path)?;
    azure_profile
        .subscriptions
        .into_iter()
        .find(|s| s.is_default)
}

fn load_azure_profile(config_path: &PathBuf) -> Option<AzureProfile> {
    let json_data = read_file(config_path).ok()?;
    let sanitized_json_data = json_data.strip_prefix('\u{feff}').unwrap_or(&json_data);
    if let Ok(azure_profile) = serde_json::from_str::<AzureProfile>(sanitized_json_data) {
        Some(azure_profile)
    } else {
        log::info!("Failed to parse azure profile.");
        None
    }
}

fn get_config_file_location(context: &Context) -> Option<PathBuf> {
    context
        .get_env("AZURE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            let mut home = context.get_home()?;
            home.push(".azure");
            Some(home)
        })
}

