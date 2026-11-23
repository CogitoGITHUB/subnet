use serde_json::Value as JsonValue;
use yaml_rust2::{Yaml, YamlLoader};

use std::borrow::Cow;
use std::env;

use super::{Context, Module, ModuleConfig};

use crate::configs::kubernetes::KubernetesConfig;
use crate::formatter::StringFormatter;
use crate::utils;

#[derive(Default)]
struct KubeCtxComponents {
    user: Option<String>,
    namespace: Option<String>,
    cluster: Option<String>,
}

fn get_current_kube_context_name<T: DataValue>(document: &T) -> Option<&str> {
    document
        .get("current-context")
        .and_then(DataValue::as_str)
        .filter(|s| !s.is_empty())
}

fn get_kube_ctx_components<T: DataValue>(
    document: &T,
    current_ctx_name: &str,
) -> Option<KubeCtxComponents> {
    document
        .get("contexts")?
        .as_array()?
        .iter()
        .find(|ctx| ctx.get("name").and_then(DataValue::as_str) == Some(current_ctx_name))
        .map(|ctx| KubeCtxComponents {
            user: ctx
                .get("context")
                .and_then(|v| v.get("user"))
                .and_then(DataValue::as_str)
                .map(String::from),
            namespace: ctx
                .get("context")
                .and_then(|v| v.get("namespace"))
                .and_then(DataValue::as_str)
                .map(String::from),
            cluster: ctx
                .get("context")
                .and_then(|v| v.get("cluster"))
                .and_then(DataValue::as_str)
                .map(String::from),
        })
}

fn get_aliased_name<'a>(
    pattern: Option<&'a str>,
    current_value: Option<&str>,
    alias: Option<&'a str>,
) -> Option<String> {
    let replacement = alias.or(current_value)?.to_string();
    let Some(pattern) = pattern else {
        // If user pattern not set, treat it as a match-all pattern
        return Some(replacement);
    };
    // If a pattern is set, but we have no value, there is no match
    let value = current_value?;
    if value == pattern {
        return Some(replacement);
    }
    let re = match regex::Regex::new(&format!("^{pattern}$")) {
        Ok(re) => re,
        Err(error) => {
            let str_pattern = format!("^{pattern}$");
            log::warn!("Could not compile regular expression `{str_pattern}`:\n{error}");
            return None;
        }
    };
    let replaced = re.replace(value, replacement.as_str());
    match replaced {
        Cow::Owned(replaced) => Some(replaced),
        // It didn't match...
        Cow::Borrowed(_) => None,
    }
}

#[derive(Debug)]
enum Document {
    Json(JsonValue),
    Yaml(Yaml),
}

trait DataValue {
    fn get(&self, key: &str) -> Option<&Self>;
    fn as_str(&self) -> Option<&str>;
    fn as_array(&self) -> Option<Vec<&Self>>;
}

impl DataValue for JsonValue {
    fn get(&self, key: &str) -> Option<&Self> {
        self.get(key)
    }

    fn as_str(&self) -> Option<&str> {
        self.as_str()
    }

    fn as_array(&self) -> Option<Vec<&Self>> {
        self.as_array().map(|arr| arr.iter().collect())
    }
}

impl DataValue for Yaml {
    fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Hash(map) => map.get(&Self::String(key.to_string())),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        self.as_str()
    }

    fn as_array(&self) -> Option<Vec<&Self>> {
        match self {
            Self::Array(arr) => Some(arr.iter().collect()),
            _ => None,
        }
    }
}

pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("kubernetes");
    let config: KubernetesConfig = KubernetesConfig::try_load(module.config);

    // As we default to disabled=true, we have to check here after loading our config module,
    // before it was only checking against whatever is in the config starship.toml
    if config.disabled {
        return None;
    }

    let have_env_config = !config.detect_env_vars.is_empty();
    let have_env_vars = have_env_config.then(|| context.detect_env_vars(&config.detect_env_vars));

    // If we have some config for doing the directory scan then we use it but if we don't then we
    // assume we should treat it like the module is enabled to preserve backward compatibility.
    let have_scan_config = [
        &config.detect_files,
        &config.detect_folders,
        &config.detect_extensions,
    ]
    .into_iter()
    .any(|v| !v.is_empty());

    let is_kube_project = have_scan_config.then(|| {
        context.try_begin_scan().is_some_and(|scanner| {
            scanner
                .set_files(&config.detect_files)
                .set_folders(&config.detect_folders)
                .set_extensions(&config.detect_extensions)
                .is_match()
        })
    });

    if !is_kube_project.or(have_env_vars).unwrap_or(true) {
        return None;
    }

    let default_config_file = context.get_home()?.join(".kube").join("config");

    let kube_cfg = context
        .get_env("KUBECONFIG")
        .unwrap_or(default_config_file.to_str()?.to_string());

    let raw_kubeconfigs = env::split_paths(&kube_cfg).map(|file| utils::read_file(file).ok());
    let kubeconfigs = parse_kubeconfigs(raw_kubeconfigs);

    let current_kube_ctx_name = kubeconfigs.iter().find_map(|v| match v {
        Document::Json(json) => get_current_kube_context_name(json),
        Document::Yaml(yaml) => get_current_kube_context_name(yaml),
    })?;

    // Even if we have multiple config files, the first key wins
    // https://kubernetes.io/docs/concepts/configuration/organize-cluster-access-kubeconfig/
    // > Never change the value or map key. ... Example: If two files specify a red-user,
    // > use only values from the first file's red-user. Even if the second file has
    // > non-conflicting entries under red-user, discard them.
    // for that reason, we can pick the first context with that name
    let ctx_components: KubeCtxComponents = kubeconfigs.iter().find_map(|kubeconfig|  match kubeconfig {
        Document::Json(json) => get_kube_ctx_components(json, current_kube_ctx_name),
        Document::Yaml(yaml) => get_kube_ctx_components(yaml, current_kube_ctx_name),
    }).unwrap_or_else(|| {
            // TODO: figure out if returning is more sensible. But currently we have tests depending on this
            log::warn!(
                "Invalid KUBECONFIG: identified current-context `{current_kube_ctx_name}`, but couldn't find the context in any config file(s): `{kube_cfg}`.\n"
                );
            KubeCtxComponents::default()
        });

    // Select the first style that matches the context_pattern and,
    // if it is defined, the user_pattern
    let (matched_context_config, display_context, display_user) = config
        .contexts
        .iter()
        .find_map(|context_config| {
            let context_alias = get_aliased_name(
                Some(context_config.context_pattern),
                Some(current_kube_ctx_name),
                context_config.context_alias,
            )?;

            let user_alias = get_aliased_name(
                context_config.user_pattern,
                ctx_components.user.as_deref(),
                context_config.user_alias,
            );
            if matches!((context_config.user_pattern, &user_alias), (Some(_), None)) {
                // defined pattern, but it didn't match
                return None;
            }

            Some((Some(context_config), context_alias, user_alias))
        })
        .unwrap_or_else(|| (None, current_kube_ctx_name.to_string(), ctx_components.user));

    // TODO: remove deprecated aliases after starship 2.0
    let display_context =
        deprecated::get_alias(display_context, &config.context_aliases, "context").unwrap();
    let display_user =
        display_user.and_then(|user| deprecated::get_alias(user, &config.user_aliases, "user"));

    let display_style = matched_context_config
        .and_then(|ctx_cfg| ctx_cfg.style)
        .unwrap_or(config.style);
    let display_symbol = matched_context_config
        .and_then(|ctx_cfg| ctx_cfg.symbol)
        .unwrap_or(config.symbol);

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|variable, _| match variable {
                "symbol" => Some(display_symbol),
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => Some(Ok(display_style)),
                _ => None,
            })
            .map(|variable| match variable {
                "context" => Some(Ok(Cow::Borrowed(display_context.as_str()))),
                "namespace" => ctx_components
                    .namespace
                    .as_ref()
                    .map(|kube_ns| Ok(Cow::Borrowed(kube_ns.as_str()))),
                "cluster" => ctx_components
                    .cluster
                    .as_ref()
                    .map(|kube_cluster| Ok(Cow::Borrowed(kube_cluster.as_str()))),
                "user" => display_user
                    .as_ref()
                    .map(|kube_user| Ok(Cow::Borrowed(kube_user.as_str()))),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `kubernetes`: \n{error}");
            return None;
        }
    });

    Some(module)
}

fn parse_kubeconfigs<I>(raw_kubeconfigs: I) -> Vec<Document>
where
    I: Iterator<Item = Option<String>>,
{
    raw_kubeconfigs
        .filter_map(|content| match content {
            Some(value) => match value.chars().next() {
                // Parsing as json is about an order of magnitude faster than parsing
                // as yaml, so do that if possible.
                Some('{') => match serde_json::from_str(&value) {
                    Ok(json) => Some(Document::Json(json)),
                    Err(_) => parse_yaml(&value),
                },
                _ => parse_yaml(&value),
            },
            _ => None,
        })
        .collect()
}

fn parse_yaml(s: &str) -> Option<Document> {
    YamlLoader::load_from_str(s)
        .ok()
        .and_then(|yaml| yaml.into_iter().next().map(Document::Yaml))
}

mod deprecated {
    use std::borrow::Cow;
    use std::collections::HashMap;

    pub fn get_alias<'a>(
        current_value: String,
        aliases: &'a HashMap<String, &'a str>,
        name: &'a str,
    ) -> Option<String> {
        let alias = if let Some(val) = aliases.get(current_value.as_str()) {
            // simple match without regex
            Some((*val).to_string())
        } else {
            // regex match
            aliases.iter().find_map(|(k, v)| {
                let re = regex::Regex::new(&format!("^{k}$")).ok()?;
                let replaced = re.replace(current_value.as_str(), *v);
                match replaced {
                    // We have a match if the replaced string is different from the original
                    Cow::Owned(replaced) => Some(replaced),
                    Cow::Borrowed(_) => None,
                }
            })
        };

        match alias {
            Some(alias) => {
                log::warn!(
                    "Usage of '{name}_aliases' is deprecated and will be removed in 2.0; Use 'contexts' with '{name}_alias' instead. (`{current_value}` -> `{alias}`)"
                );
                Some(alias)
            }
            None => Some(current_value),
        }
    }
}

"#,
        )?;
        file.sync_all()?;

        let actual = ModuleRenderer::new("kubernetes")
            .path(dir.path())
            .env("KUBECONFIG", filename.to_string_lossy().as_ref())
            .config(toml::toml! {
                [kubernetes]
                disabled = false
                format = "($user )($context )($cluster )($namespace)"
            })
            .collect();

        let expected = Some("test_user test_context test_namespace".to_string());
        assert_eq!(expected, actual);
        dir.close()
    }
}
