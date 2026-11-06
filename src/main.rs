use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::prelude::*;
use std::fs;
use reqwest::header::HeaderMap;
use std::env;

fn create_file_if_not_exists(file_name: &str) -> Result<File, Box<dyn std::error::Error>> {
    Ok(fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(file_name)?)
}

async fn update_ip(file: &mut File, ip: &str, old_ip: &str, valid_record_types: HashSet<String>) -> Result<(), Box<dyn std::error::Error>> {
    update_ip_file(file, ip)?;
    for domain_name in DOMAIN_NAMES {
        update_digital_ocean(ip, old_ip, domain_name, &valid_record_types).await?;
    }
    Ok(())
}

fn update_ip_file(file: &mut File, ip: &str) -> Result<(), Box<dyn std::error::Error>> {
    file.write_all(ip.as_bytes())?;
    Ok(())
}

async fn update_digital_ocean(ip: &str, old_ip: &str, domain_name: &str, valid_record_types: &HashSet<String>) -> Result<(), Box<dyn std::error::Error>> {
    let token = env::var(TOKEN_VARIABLE_NAME);
    if token.is_err() {
        println!("No token found in environment variables");
        return Ok(());
    }

    let token = token.unwrap();

    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    headers.insert("Authorization", format!("Bearer {token}").parse().unwrap());

    let resp = client.get(format!("{API_BASE_URL}/{domain_name}/records"))
        .headers(headers.clone())
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    headers.insert("Content-Type", "application/json".parse().unwrap());
    let resp_obj = resp.as_object().expect("Response is not an object");
    let records = resp_obj.get("domain_records").expect("No domain_records key in response").as_array().expect("domain_records is not an array");
    for record in records {
        let record = record.as_object().expect("Record is not an object");
        let record_ip = record.get("data").expect("No data key in record").as_str().expect("data key is not a string").trim();
        let record_id = record.get("id").expect("No id key in record").as_u64().expect("id key is not a number");
        let record_type = record.get("type").expect("No type key in record").as_str().expect("type key is not a string").trim();
        if !valid_record_types.contains(record_type) {
            println!("Skipping record for {domain_name} with type {record_type} because it is not recognized as valid");
            continue;
        }
        update_record(&client, &headers, record_type, &record_id.to_string(), ip, domain_name).await?;
    }

    Ok(())
}

async fn update_record(client: &reqwest::Client, headers: &HeaderMap, record_type: &str, record_id: &str, ip: &str, domain_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Updating record {record_id} with type {record_type} for domain {domain_name}");
    let mut body = HashMap::new();
    body.insert("data", ip);
    body.insert("type", record_type);
    let res = client
        .patch(format!("{API_BASE_URL}/{domain_name}/records/{record_id}"))
        .headers(headers.clone())
        .json(&body)
        .send()
        .await?
        .text()
        .await?;
    println!("Response: {res}");
    Ok(())
}

async fn get_ip() -> Result<String, Box<dyn std::error::Error>> {
    let resp = reqwest::get("https://httpbin.org/ip")
        .await?
        .json::<HashMap<String, String>>()
        .await?;
    Ok(resp.get("origin").expect("No origin key in response").to_string())
}

const TOKEN_VARIABLE_NAME: &str = "DIGITALOCEAN_TOKEN";
const FILE_NAME: &str = "ip.txt";
const DOMAIN_NAMES: [&str; 2] = ["karschner.studio", "ekars.ch"];
const API_BASE_URL: &str = "https://api.digitalocean.com/v2/domains";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    let ip = get_ip().await?;
    println!("Current IP address is: {}", ip);
    let old_ip = fs::read_to_string(FILE_NAME);
    let old_ip = old_ip.unwrap_or("".to_string()).trim().to_string();
    println!("old ip: {old_ip}");
    if old_ip != *ip || (args.len() > 1 && &args[1] == "--force-update") {
        if old_ip == *ip {
            println!("old ip matches current, forcing update due to --force-update flag");
        }
        let mut file = create_file_if_not_exists(FILE_NAME)?;

        let valid_record_types = get_valid_record_types();
        update_ip(&mut file, &ip, &old_ip, valid_record_types).await?;
    }

    Ok(())
}

// List valid record types to target for updates
fn get_valid_record_types() -> HashSet<String> {
    HashSet::from(["A".to_string(), "@".to_string()])
}
