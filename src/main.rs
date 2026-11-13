use std::collections::HashMap;
use std::fs;
use dynamic_dns::modules::{self, digitalocean, routeros};
use serde::Deserialize;
use std::env::VarError;
use dynamic_dns::core::{IpValidationMode, Module};

const CONFIG_PATH: &str = "config.toml";
async fn get_ip() -> Result<String, Box<dyn std::error::Error>> {
    let resp = reqwest::get("https://httpbin.org/ip")
        .await?
        .json::<HashMap<String, String>>()
        .await?;
    Ok(resp.get("origin").expect("No origin key in response").to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = match read_config(CONFIG_PATH) {
        Ok(c) => c,
        Err(e) => return Err(e.get_err())
    };
    let mods: Vec<modules::ModuleEnum> = config.modules.into_iter().map(|module_config| {
        modules::ModuleEnum::new(module_config)
    }).collect();
    let ip = get_ip().await?;

    let options = dynamic_dns::core::HandleIpOptions {
        ip_validation_mode: IpValidationMode::Local,
    };

    for mut module in mods {
        module.handle_ip(ip.clone(), options.clone()).await.map_err(|e| eprintln!("{:?}", e));
    }

    Ok(())
}

#[derive(Deserialize)]
struct Config {
    modules: Vec<modules::ModuleConfig>,
}

enum ConfigReadError {
    IoError(std::io::Error),
    DeserializationError(toml::de::Error),
    ShellExpandError(shellexpand::LookupError<VarError>),
}

impl ConfigReadError {
    fn get_err(self) -> Box<dyn std::error::Error> {
        match self {
            Self::IoError(e) => Box::new(e),
            Self::DeserializationError(e) => Box::new(e),
            Self::ShellExpandError(e) => Box::new(e),
        }
    }
}

impl From<std::io::Error> for ConfigReadError {
    fn from(err: std::io::Error) -> Self {
        Self::IoError(err)
    }
}

impl From<toml::de::Error> for ConfigReadError {
    fn from(err: toml::de::Error) -> Self {
        Self::DeserializationError(err)
    }
}

impl From<shellexpand::LookupError<VarError>> for ConfigReadError {
    fn from(err: shellexpand::LookupError<VarError>) -> Self {
        Self::ShellExpandError(err)
    }
}

fn read_config(file_path: &str) -> Result<Config, ConfigReadError> {
    let s = fs::read_to_string(file_path)?;
    let s = shellexpand::env(&s)?;
    Ok(toml::from_str(&s)?)
}
