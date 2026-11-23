use super::{Context, Module};

#[cfg(not(target_os = "linux"))]
pub fn module<'a>(_context: &'a Context) -> Option<Module<'a>> {
    None
}

#[cfg(target_os = "linux")]
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    use super::ModuleConfig;
    use crate::configs::container::ContainerConfig;
    use crate::formatter::StringFormatter;
    use crate::utils::{self, read_file};

    pub fn container_name(context: &Context) -> Option<String> {
        use crate::utils::context_path;

        if context_path(context, "/proc/vz").exists() && !context_path(context, "/proc/bc").exists()
        {
            // OpenVZ
            return Some("OpenVZ".into());
        }

        if context_path(context, "/run/host/container-manager").exists() {
            // OCI
            return Some("OCI".into());
        }

        if context_path(context, "/dev/incus/sock").exists() {
            // Incus
            return Some("Incus".into());
        }

        let container_env_path = context_path(context, "/run/.containerenv");
        if container_env_path.exists() {
            // podman and others

            let image_res = read_file(container_env_path)
                .map(|s| {
                    s.lines()
                        .find_map(|l| {
                            if let Some(name_val) = l.strip_prefix("name=\"") {
                                return name_val.strip_suffix('"').map(|n| n.to_string());
                            }

                            l.starts_with("image=\"").then(|| {
                                let r = l.split_at(7).1;
                                let name = r.rfind('/').map(|n| r.split_at(n + 1).1);
                                String::from(name.unwrap_or(r).trim_end_matches('"'))
                            })
                        })
                        .unwrap_or_else(|| "podman".into())
                })
                .unwrap_or_else(|_| "podman".into());

            return Some(image_res);
        }

        // WSL with systemd will set the contents of this file to "wsl"
        // Avoid showing the container module in that case
        // Honor the contents of this file if "docker" and not running in podman or wsl
        let systemd_path = context_path(context, "/run/systemd/container");
        if let Ok(s) = utils::read_file(systemd_path) {
            match s.trim() {
                "docker" => return Some("Docker".into()),
                "wsl" => (),
                _ => return Some("Systemd".into()),
            }
        }

        if context_path(context, "/.dockerenv").exists() {
            // docker
            return Some("Docker".into());
        }

        None
    }

    let mut module = context.new_module("container");
    let config: ContainerConfig = ContainerConfig::try_load(module.config);

    if config.disabled {
        return None;
    }

    let container_name = container_name(context)?;

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
                "name" => Some(Ok(&container_name)),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `container`: \n{error}");
            return None;
        }
    });

    Some(module)
}

