pub mod core {
    use std::{fs, path};

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
        async fn handle_ip(&mut self, new_ip: String, options: HandleIpOptions) -> Result<HandleIpSuccess, HandleIpError>;
        fn is_debug(&self) -> bool;
        fn debug(&self, text: &str) {
            if self.is_debug() {
                println!("{text}");
            }
        }
        fn get_ip_path(&self) -> Option<&path::PathBuf>;
        fn local_ip_path_property_name(&self) -> String;
        fn get_local_ip(&self) -> Result<String, HandleIpError> {
            let ip_storage_path = if let Some(path) = self.get_ip_path() {
                path
            } else {
                return Err(HandleIpError::ConfigError(format!("{} be set when using IpValidationMode::Local", self.local_ip_path_property_name())))
            };
            let path = fs::read_to_string(ip_storage_path)?;
            Ok(path)
        }
        fn is_local_ip_matching(&self, new_ip: &str) -> Result<bool, HandleIpError> {
            Ok(self.get_local_ip()?.eq(new_ip))
        }
    }
}

pub mod routeros {
    use std::{io::{Read, Write}, net, str::Utf8Error};

    pub struct RouterOSSocket {
        socket: net::TcpStream,
        username: String,
        password: String,
    }

    pub enum SendCommandError {
        IoError(std::io::Error),
        MaxLengthError(String),
        AuthError(String),
        UtfError(Utf8Error),
        SequenceError(String),
    }

    impl From<std::io::Error> for SendCommandError {
        fn from(e: std::io::Error) -> Self {
            Self::IoError(e)
        }
    }

    impl From<Utf8Error> for SendCommandError {
        fn from(e: Utf8Error) -> Self {
            Self::UtfError(e)
        }
    }

    impl RouterOSSocket {
        pub fn new(args: RouterOSSocketArgs) -> Result<Self, SendCommandError> {
            let socket = net::TcpStream::connect(format!("{}:{}", args.ip, args.port))?;
            let mut res = Self {
                socket,
                username: args.username,
                password: args.password,
            };
            res.login()?;
            Ok(res)
        }

        fn make_login_command(&self) -> Vec<String> {
            vec!["/login".to_string(), format!("=name={}", self.username), format!("=password={}", self.password)]
        }

        fn login(&mut self) -> Result<(), SendCommandError> {
            let login_cmd = self.make_login_command();
            self.send_command(login_cmd.iter().map(|s| s.as_str()).collect())?;
            let response = self.read_sentence()?;
            if !response[0].eq("!done") {
                return Err(SendCommandError::AuthError(response.join("\n")));
            }
            Ok(())
        }

        pub fn send_command(&mut self, words: Vec<&str>) -> Result<(), SendCommandError> {
            for word in words {
                self.socket.write(&Self::encode_word(word)?)?;
            }
            self.socket.write(&Self::encode_word("")?)?;
            Ok(())
        }

        fn encode_word(word: &str) -> Result<Vec<u8>, SendCommandError> {
            let bytes = word.as_bytes();
            let len_bytes = bytes.len().to_be_bytes();
            let encoded_len = match bytes.len() {
                l if l < 0x80 => &len_bytes[len_bytes.len() - 1..],
                l if l < 0x4000 => &len_bytes[len_bytes.len() - 2..],
                l if l < 0x200000 => &len_bytes[len_bytes.len() - 3..],
                l if l < 0x10000000 => &len_bytes[len_bytes.len() - 4..],
                l if l < 0x100000000 => &[&[0xF0], &len_bytes[len_bytes.len() - 4..]].concat(),
                _ => return Err(SendCommandError::MaxLengthError(word.to_string())),
            };
            Ok([&encoded_len, bytes].concat())
        }

        pub fn read_sentence(&mut self) -> Result<Vec<String>, SendCommandError> {
            let mut words = Vec::new();
            loop {
                let word = self.read_word()?;
                words.push(word);
                if words[words.len()-1].eq("!done") {
                    break;
                }
            }
            Ok(words)
        }

        fn read_word(&mut self) -> Result<String, SendCommandError> {
            let socket = &mut self.socket;
            let mut c_buf = [0u8; 1];
            socket.read_exact(&mut c_buf)?;
            println!("{:?}", c_buf);
            let c = c_buf[0];
            let length = if c & 0x80 == 0x00 {
                c as usize
            } else if c & 0xC0 == 0x80 {
                let mut len_buf = [0u8; 1];
                socket.read_exact(&mut len_buf)?;
                (((c & 0x3F) as usize) << 8) + len_buf[0] as usize
            } else if c & 0xE0 == 0xC0{
                let mut len_buf = [0u8; 2];
                socket.read_exact(&mut len_buf)?;
                (((c & 0x1F) as usize) << 16) + ((len_buf[1] as usize) << 8) + len_buf[0] as usize
            } else if c & 0xF0 == 0xE0 {
                let mut len_buf = [0u8; 3];
                socket.read_exact(&mut len_buf)?;
                (((c & 0x0F) as usize) << 24) + ((len_buf[2] as usize) << 16) + ((len_buf[1] as usize) << 8) + len_buf[0] as usize
            } else {
                let mut len_buf = [0u8; 4];
                socket.read_exact(&mut len_buf)?;
                ((len_buf[3] as usize) << 24) + ((len_buf[2] as usize) << 16) + ((len_buf[1] as usize) << 8) + len_buf[0] as usize
            };

            let mut word_buf = vec![0u8; length];
            socket.read_exact(&mut word_buf)?;

            println!("{:?}", word_buf);
            let word = str::from_utf8(&word_buf)?;
            Ok(word.to_owned())
        }

    }

    pub struct RouterOSSocketArgs {
        pub ip: String,
        pub username: String,
        pub password: String,
        pub port: u16,
    }
}

pub mod modules {
    use serde::Deserialize;

    use crate::core::Module;
    #[derive(Deserialize)]
    #[serde(tag = "type")]
    pub enum ModuleConfig {
        #[serde(rename = "digitalocean")]
        DigitalOcean(digitalocean::DigitaloceanConfig),
        #[serde(rename = "routeros")]
        RouterOS(routeros::RouterOSConfig),
    }

    pub enum ModuleEnum {
        DigitalOcean(digitalocean::DigitaloceanModule),
        RouterOS(routeros::RouterOSModule),
    }

    impl Module for ModuleEnum {
        type Config = ModuleConfig;

        fn new(config: Self::Config) -> Self {
            match config {
                Self::Config::DigitalOcean(cfg) => Self::DigitalOcean(digitalocean::DigitaloceanModule::new(cfg)),
                Self::Config::RouterOS(cfg) => Self::RouterOS(routeros::RouterOSModule::new(cfg)),
            }
        }

        async fn handle_ip(&mut self, new_ip: String, options: crate::core::HandleIpOptions) -> Result<crate::core::HandleIpSuccess, crate::core::HandleIpError> {
            match self {
                Self::DigitalOcean(m) => m.handle_ip(new_ip, options).await,
                Self::RouterOS(m) => m.handle_ip(new_ip, options).await,
            }
        }

        fn is_debug(&self) -> bool {
            match self {
                Self::DigitalOcean(m) => m.is_debug(),
                Self::RouterOS(m) => m.is_debug(),
            }
        }

        fn get_ip_path(&self) -> Option<&std::path::PathBuf> {
            match self {
                Self::DigitalOcean(m) => m.get_ip_path(),
                Self::RouterOS(m) => m.get_ip_path(),
            }
        }

        fn local_ip_path_property_name(&self) -> String {
            match self {
                Self::DigitalOcean(m) => m.local_ip_path_property_name(),
                Self::RouterOS(m) => m.local_ip_path_property_name(),
            }
        }
    }

    pub mod routeros {

        use std::path;

use serde::Deserialize;
        
        use crate::{core::{self, Module}, routeros::SendCommandError};

        #[derive(Deserialize)]
        pub struct RouterOSConfig {
            router_ip: String,
            username: String,
            password: String,
            #[serde(default="default_port")]
            port: u16,
            #[serde(default)]
            debug: bool,
            #[serde(default)]
            local_ip_path: Option<path::PathBuf>,
            comment_substr: String,
        }

        fn default_port() -> u16 {
            8728
        }

        pub struct RouterOSModule {
            // Lazily initialized; logged in on initialization
            socket: Option<crate::routeros::RouterOSSocket>,
            config: RouterOSConfig,
        }

        impl RouterOSModule {
            fn get_socket(&mut self) -> Result<&mut crate::routeros::RouterOSSocket, SendCommandError> {
                if self.socket.is_none() {
                    let s = crate::routeros::RouterOSSocket::new(crate::routeros::RouterOSSocketArgs {
                        ip: self.config.router_ip.clone(),
                        username: self.config.username.clone(),
                        password: self.config.password.clone(),
                        port: self.config.port,
                    })?;
                    self.socket = Some(s);
                }

                Ok(self.socket.as_mut().unwrap())
            }

            fn find_matching_records(&mut self, comment_substr: &str) -> Result<Vec<Record>, SendCommandError> {
                let records = self.get_records()?;
                Ok(records.into_iter().filter(|record| record.comment.as_ref().is_some_and(|comment| comment.contains(comment_substr))).collect())
            }

            fn update_record(&mut self, record: Record, new_ip: &str) -> Result<(), SendCommandError> {
                let socket = self.get_socket()?;
                let id_str = format!("=.id={}", record.id);
                let dst_str = format!("=dst-address={new_ip}");
                let update_command = vec!["/ip/firewall/nat/set", &id_str, &dst_str];
                socket.send_command(update_command)?;
                self.debug(&format!("Updated record {:?}", record));
                Ok(())
            }

            fn get_records(&mut self) -> Result<Vec<Record>, SendCommandError> {
                let socket = self.get_socket()?;
                socket.send_command(vec!["/ip/firewall/nat/print"])?;
                let words = socket.read_sentence()?;
                let mut i = 0;
                let mut records = Vec::new();
                while i < words.len() {
                    println!("i: {i}, words length: {}", words.len());
                    if words[i].eq("") {
                        i += 1;
                        continue;
                    }
                    if words[i].eq("!done") {
                        break;
                    }
                    if !words[i].eq("!re") {
                        return Err(SendCommandError::SequenceError(format!("Unexpected token: Expected \"!re\" or \"!done\", found {}", words[i])));
                    }
                    i += 1;

                    let mut record = Record {
                        comment: None,
                        id: "".to_string(),
                        dst_address: None,
                    };


                    while i < words.len() {
                        if words[i].starts_with("!") || words[i].eq("") {
                            break;
                        }
                        println!("{:?}", words[i]);
                        let parts: Vec<&str> = words[i].splitn(3, "=").collect();
                        println!("{:?}", parts);
                        if parts[1] == "comment" {
                            record.comment = Some(parts[2].to_string());
                        } else if parts[1] == "dst-address" {
                            record.dst_address = Some(parts[2].to_string());
                        } else if parts[1] == ".id" {
                            record.id = parts[2].to_string();
                        }

                        i += 1;
                    }

                    if record.id != "" {
                        records.push(record);
                    }
                }
                Ok(records)           
            }
        }

        impl From<SendCommandError> for core::HandleIpError {
            fn from(e: SendCommandError) -> Self {
                match e {
                    SendCommandError::IoError(error) => core::HandleIpError::IoError(error),
                    SendCommandError::MaxLengthError(cmd) => core::HandleIpError::GenericError(format!("Command too long: {cmd}")),
                    SendCommandError::AuthError(e) => core::HandleIpError::ConfigError(format!("RouterOS Authentication error: {e}")),
                    SendCommandError::UtfError(e) => core::HandleIpError::GenericError(format!("Error reading bytes from router: {}", e.to_string())),
                    SendCommandError::SequenceError(e) => core::HandleIpError::GenericError(e),
                }
            }
        }

        #[derive(Debug)]
        struct Record {
            id: String,
            dst_address: Option<String>,
            comment: Option<String>,
        }

        impl core::Module for RouterOSModule {
            type Config = RouterOSConfig;

            fn is_debug(&self) -> bool {
                self.config.debug
            }

            fn new(config: Self::Config) -> Self {
                let socket = None;
                Self {
                    socket,
                    config,
                }
            }

            async fn handle_ip(&mut self, new_ip: String, options: core::HandleIpOptions) -> Result<core::HandleIpSuccess, core::HandleIpError> {
                if options.ip_validation_mode == core::IpValidationMode::Local && self.is_local_ip_matching(&new_ip)? {
                    return Ok(core::HandleIpSuccess::NoUpdateNecessary);
                }
                let comment_substr = self.config.comment_substr.clone();
                let records = self.find_matching_records(&comment_substr)?;
                self.debug(&format!("Matching Records: {:?}", records));
                let mut updated = 0;
                for record in records {
                    if options.ip_validation_mode == core::IpValidationMode::Actual && record.dst_address.as_ref().is_none_or(|it| new_ip.eq(it)) {
                        self.update_record(record, &new_ip)?;
                        updated += 1;
                    }
                }
                Ok(core::HandleIpSuccess::Updated(updated))
            }

            fn get_ip_path(&self) -> Option<&std::path::PathBuf> {
                self.config.local_ip_path.as_ref()
            }

            fn local_ip_path_property_name(&self) -> String {
                "config.local_ip_path".to_string()
            }
        }
    }

    pub mod digitalocean {
        use std::{collections::{HashMap, HashSet}, fs::{self, File}, io::Write, path::PathBuf};
        use reqwest::header::HeaderMap;
        use serde::Deserialize;

        use crate::core::{HandleIpError, HandleIpOptions, HandleIpSuccess, IpValidationMode, Module};

        const API_BASE_URL: &str = "https://api.digitalocean.com/v2/domains";

        #[derive(Deserialize)]
        pub struct DigitaloceanConfig {
            #[serde(default)]
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

            fn is_debug(&self) -> bool {
                self.config.debug
            }

            fn new(config: Self::Config) -> Self {
                let http_client = reqwest::Client::new();
                return DigitaloceanModule {
                    config,
                    http_client,

                }
            }

            async fn handle_ip(&mut self, new_ip: String, options: HandleIpOptions) -> Result<HandleIpSuccess, HandleIpError> {
                if options.ip_validation_mode == IpValidationMode::Local && self.is_local_ip_matching(&new_ip)? {
                    return Ok(HandleIpSuccess::NoUpdateNecessary);
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

            fn get_ip_path(&self) -> Option<&std::path::PathBuf> {
                self.config.ip_storage_path.as_ref()
            }

            fn local_ip_path_property_name(&self) -> String {
                "config.ip_storage_path".to_string()
            }
        }
    }
}
