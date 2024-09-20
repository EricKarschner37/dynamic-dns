use std::collections::HashMap;
use std::fs::File;
use std::io::prelude::*;
use std::fs;
use reqwest::header::HeaderMap;
use std::env;

fn create_file_if_not_exists(file_name: &str) -> Result<File, Box<dyn std::error::Error>> {
    Ok(fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(file_name)?)
}

async fn update_ip(file: &mut File, ip: &str, old_ip: &str) -> Result<(), Box<dyn std::error::Error>> {
    update_ip_file(file, ip)?;
    update_digital_ocean(ip, old_ip).await?;
    Ok(())
}

fn update_ip_file(file: &mut File, ip: &str) -> Result<(), Box<dyn std::error::Error>> {
    file.write_all(ip.as_bytes())?;
    Ok(())
}

async fn update_digital_ocean(ip: &str, old_ip: &str) -> Result<(), Box<dyn std::error::Error>> {
    let token = env::var(TOKEN_VARIABLE_NAME);
    if token.is_err() {
        println!("No token found in environment variables");
        return Ok(());
    }

    let token = token.unwrap();

    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    headers.insert("Authorization", format!("Bearer {token}").parse().unwrap());

    let resp = client.get(format!("{API_BASE_URL}/{DOMAIN_NAME}/records"))
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
        let b = old_ip == record_ip;
        let c = record_ip == "75.214.244.204";
        println!("old ip: {old_ip}, record ip: {record_ip}, === {b}, === {c}");
        let old_ip_bytes = old_ip.as_bytes();
        let record_ip_bytes = record_ip.as_bytes();
        // println!("old ip bytes: {old_ip_bytes:#?}, record ip bytes: {record_ip_bytes:#?}");
        if old_ip == record_ip {
            update_record(&client, &headers, record_type, &record_id.to_string(), ip).await?;
        }
    }

    Ok(())
}

async fn update_record(client: &reqwest::Client, headers: &HeaderMap, record_type: &str, record_id: &str, ip: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut body = HashMap::new();
    body.insert("data", ip);
    body.insert("type", record_type);
    let res = client
        .patch(format!("{API_BASE_URL}/{DOMAIN_NAME}/records/{record_id}"))
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

const TOKEN_VARIABLE_NAME: &str = "DIGITAL_OCEAN_TOKEN";
const FILE_NAME: &str = "ip.txt";
const DOMAIN_NAME: &str = "karschner.studio";
const API_BASE_URL: &str = "https://api.digitalocean.com/v2/domains";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ip = get_ip().await?;
    println!("Current IP address is: {}", ip);
    let old_ip = fs::read_to_string(FILE_NAME)?;
    println!("old ip: {old_ip}");
    if old_ip != *ip {
        let mut file = create_file_if_not_exists(FILE_NAME)?;
        update_ip(&mut file, &ip, &old_ip).await?;
    }

    Ok(())
}
