use ini::Ini;
use std::borrow::Cow;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{LazyLock, OnceLock};

use super::{Context, Module, ModuleConfig};

use crate::configs::gcloud::GcloudConfig;
use crate::formatter::StringFormatter;
use crate::utils;

type Account<'a> = (&'a str, Option<&'a str>);

struct GcloudContext {
    config_name: String,
    config_path: PathBuf,
    config: OnceLock<Option<Ini>>,
}

impl<'a> GcloudContext {
    pub fn new(config_name: &str, config_path: &Path) -> Self {
        Self {
            config_name: config_name.to_string(),
            config_path: PathBuf::from(config_path),
            config: OnceLock::default(),
        }
    }

    fn get_config(&self) -> Option<&Ini> {
        self.config
            .get_or_init(|| Ini::load_from_file(&self.config_path).ok())
            .as_ref()
    }

    pub fn get_account(&'a self) -> Option<Account<'a>> {
        let config = self.get_config()?;
        let account = config.section(Some("core"))?.get("account")?;
        let mut segments = account.splitn(2, '@');
        Some((segments.next()?, segments.next()))
    }

    pub fn get_project(&'a self) -> Option<&'a str> {
        let config = self.get_config()?;
        config.section(Some("core"))?.get("project")
    }

    pub fn get_region(&'a self) -> Option<&'a str> {
        let config = self.get_config()?;
        config.section(Some("compute"))?.get("region")
    }
}

fn get_current_config(context: &Context) -> Option<(String, PathBuf)> {
    let config_dir = get_config_dir(context)?;
    let name = get_active_config(context, &config_dir)?;
    let path = config_dir
        .join("configurations")
        .join(format!("config_{name}"));
    Some((name, path))
}

fn get_config_dir(context: &Context) -> Option<PathBuf> {
    context
        .get_env("CLOUDSDK_CONFIG")
        .map(PathBuf::from)
        .or_else(|| {
            let home = context.get_home()?;
            Some(home.join(".config").join("gcloud"))
        })
}

fn get_active_config(context: &Context, config_dir: &Path) -> Option<String> {
    context.get_env("CLOUDSDK_ACTIVE_CONFIG_NAME").or_else(|| {
        let path = config_dir.join("active_config");
        utils::read_file(path)
            .ok()?
            .lines()
            .next()
            .map(String::from)
    })
}

pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("gcloud");
    let config: GcloudConfig = GcloudConfig::try_load(module.config);

    if !(context.detect_env_vars(&config.detect_env_vars)) {
        return None;
    }

    let (config_name, config_path) = get_current_config(context)?;

    if config_name == "NONE" {
        return None;
    }
    let gcloud_context = GcloudContext::new(&config_name, &config_path);
    let account: LazyLock<Option<Account<'_>>, _> = LazyLock::new(|| gcloud_context.get_account());

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
                "account" => account
                    .map(|(account, _)| account)
                    .map(Cow::Borrowed)
                    .map(Ok),
                "domain" => account
                    .and_then(|(_, domain)| domain)
                    .map(Cow::Borrowed)
                    .map(Ok),
                "region" => context
                    .get_env("CLOUDSDK_COMPUTE_REGION")
                    .map(Cow::Owned)
                    .or_else(|| gcloud_context.get_region().map(Cow::Borrowed))
                    .map(|region| {
                        config
                            .region_aliases
                            .get(region.as_ref())
                            .copied()
                            .map_or(region, Cow::Borrowed)
                    })
                    .map(Ok),
                "project" => context
                    .get_env("CLOUDSDK_CORE_PROJECT")
                    .map(Cow::Owned)
                    .or_else(|| gcloud_context.get_project().map(Cow::Borrowed))
                    .map(|project| {
                        config
                            .project_aliases
                            .get(project.as_ref())
                            .copied()
                            .map_or(project, Cow::Borrowed)
                    })
                    .map(Ok),
                "active" => Some(Ok(Cow::Borrowed(&gcloud_context.config_name))),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::error!("Error in module `gcloud`: \n{error}");
            return None;
        }
    });

    Some(module)
}

