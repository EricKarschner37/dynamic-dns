pub mod core {
    #[derive(Eq, PartialEq, Clone)]
    pub enum IpValidationMode {
        None,
        Local,
        Actual,
    }

    #[derive(Clone)]
    pub struct HandleIpOptions {
        pub ip_validation_mode: IpValidationMode,
    }

    #[derive(Debug)]
    pub enum HandleIpError {
        GenericError(String),
        ConfigError(String),
        IoError(std::io::Error),
        NetworkError(reqwest::Error),
    }

    pub enum HandleIpSuccess {
        Updated(u32),
        NoUpdateNecessary,
    }

    impl From<std::io::Error> for HandleIpError {
        fn from(err: std::io::Error) -> Self {
            Self::IoError(err)
        }
    }

    impl From<reqwest::Error> for HandleIpError {
        fn from(err: reqwest::Error) -> Self {
            Self::NetworkError(err)
        }
    }

    pub trait Module {
        type Config;
        fn new(config: Self::Config) -> Self;
        async fn handle_ip(&self, new_ip: String, options: HandleIpOptions) -> Result<HandleIpSuccess, HandleIpError>;
    }

}

pub mod modules {
    use serde::Deserialize;
    #[derive(Deserialize)]
    #[serde(tag = "type")]
    pub enum ModuleConfig {
        #[serde(rename = "digitalocean")]
        DigitalOcean(digitalocean::DigitaloceanConfig),
    }

    pub mod digitalocean {
        use std::{collections::{HashMap, HashSet}, fs::{self, File}, io::Write, path::PathBuf};
        use reqwest::header::HeaderMap;
        use serde::Deserialize;

        use crate::core::{HandleIpError, HandleIpOptions, HandleIpSuccess, IpValidationMode};

        const API_BASE_URL: &str = "https://api.digitalocean.com/v2/domains";

        #[derive(Deserialize)]
        pub struct DigitaloceanConfig {
            ip_storage_path: Option<PathBuf>,
            api_token: String,
            domain_names: Vec<String>,
            valid_record_types: HashSet<String>,
            #[serde(default)]
            debug: bool,
        }

        pub struct DigitaloceanModule {
            config: DigitaloceanConfig,
            http_client: reqwest::Client,
        }

        struct UpdateRecordArgs<'a> {
            ip: &'a str,
            record_type: &'a str,
            domain_name: &'a str,
            record_id: u64,
        }

        struct GetRecordsArgs<'a> {
            domain_name: &'a str,
        }

        #[derive(Deserialize)]
        struct GetRecordsResponse {
            domain_records: Vec<Record>,
        }
        
        #[derive(Deserialize)]
        struct Record {
            id: u64,
            #[serde(rename = "type")]
            record_type: String,
            #[serde(rename = "data")]
            ip: String,
        }

        impl DigitaloceanModule {
            fn debug(&self, text: &str) {
                if self.config.debug {
                    println!("{text}");
                }
            }

            fn make_headers(&self, content_type: Option<&str>) -> HeaderMap {
                let mut headers = HeaderMap::new();
                headers.insert("Authorization", format!("Bearer {}", self.config.api_token).parse().unwrap());
                if let Some(content_type) = content_type {
                    headers.insert("Content-Type", content_type.parse().unwrap());
                }

                headers
            }

            async fn update_record(&self, args: UpdateRecordArgs<'_>) -> Result<(), reqwest::Error> {
                let mut body = HashMap::new();
                body.insert("data", args.ip);
                body.insert("type", args.record_type);
                self.http_client
                    .patch(format!("{API_BASE_URL}/{}/records/{}", args.domain_name, args.record_id))
                    .headers(self.make_headers(Some("application/json")))
                    .send()
                    .await?;

                Ok(())
            }

            async fn get_records(&self, args: GetRecordsArgs<'_>) -> Result<Vec<Record>, reqwest::Error> {
                self.debug(&format!("Fetching records for {}", args.domain_name));
                let resp = self.http_client
                    .get(format!("{API_BASE_URL}/{}/records", args.domain_name))
                    .headers(self.make_headers(None))
                    .send()
                    .await?
                    .json::<GetRecordsResponse>()
                    .await?;
                Ok(resp.domain_records)
            }

            fn create_file_if_not_exists(path: &PathBuf) -> Result<File, std::io::Error> {
                fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(path)
            }

            fn write_ip_to_file(&self, new_ip: &str) -> Result<(), std::io::Error> {
                let Some(ip_storage_path) = &self.config.ip_storage_path else {
                    return Ok(());
                };

                let mut file = Self::create_file_if_not_exists(ip_storage_path)?;
                file.write_all(new_ip.as_bytes())
            }
        }

        impl crate::core::Module for DigitaloceanModule {
            type Config = DigitaloceanConfig;

            fn new(config: Self::Config) -> Self {
                let http_client = reqwest::Client::new();
                return DigitaloceanModule {
                    config,
                    http_client,

                }
            }

            async fn handle_ip(&self, new_ip: String, options: HandleIpOptions) -> Result<HandleIpSuccess, HandleIpError> {
                if options.ip_validation_mode == IpValidationMode::Local {
                    let ip_storage_path = if let Some(path) = self.config.ip_storage_path.as_ref() {
                        path
                    } else {
                        return Err(HandleIpError::ConfigError("config.ip_storage_path must be set when using IpValidationMode::Local".to_string()))
                    };
                    let old_ip = fs::read_to_string(ip_storage_path)?;
                    if old_ip.eq(&new_ip) {
                        return Ok(HandleIpSuccess::NoUpdateNecessary);
                    }
                }

                let mut updated = 0;

                for domain_name in &self.config.domain_names {
                    let records = self.get_records(GetRecordsArgs{
                        domain_name,
                    }).await?;
                    for record in records {
                        if !self.config.valid_record_types.contains(&record.record_type) {
                            self.debug(&format!("Skipping record with type {} in {domain_name} since it is not a configured type", &record.record_type));
                            continue;
                        }

                        if options.ip_validation_mode != IpValidationMode::Actual || record.ip != new_ip {
                            self.update_record(UpdateRecordArgs{
                                domain_name,
                                ip: &new_ip,
                                record_type: &record.record_type,
                                record_id: record.id,
                            }).await?;
                            self.debug(&format!("Updated record with type {} in {domain_name} from {} to {new_ip}", record.record_type, record.ip));
                            updated += 1;
                        }
                    }
                }

                self.write_ip_to_file(&new_ip)?;

                Ok(HandleIpSuccess::Updated(updated))
            }
        }
    }
}
