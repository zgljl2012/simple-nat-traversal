use std::{collections::HashMap, time::Duration};

use log::{debug, info};
use reqwest::ClientBuilder;

use crate::message::Message;

const GATEWAY_TIMEOUT: u64 = 20;

#[derive(Debug, Clone)]
pub struct RequestLine {
    pub method: String,
    pub url: String,
    pub http_version: String,
}

impl RequestLine {
    pub fn from_utf8(line: &str) -> Self {
        let mut r = line.trim().split_whitespace();
        Self {
            method: r.next().unwrap_or("GET").to_string(),
            url: r.next().unwrap_or("Unknown").to_string(),
            http_version: r.next().unwrap_or("HTTP/1.1").to_string(),
        }
    }
	pub fn has_body(&self) -> bool {
		["POST", "PUT", "PATCH"].contains(&self.method.to_uppercase().as_str())
	}
	pub fn is_supported(&self) -> bool {
		["GET", "POST"].contains(&self.method.to_uppercase().as_str())
	}
}

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub request_line: RequestLine,
    pub host: String,
    pub user_agent: String,
    pub accept: String,
    pub proxy_connection: String,
	pub data: Option<String>,
}

impl HttpRequest {
    pub fn from_utf8(text: &str) -> Self {
        let mut it = text.lines();
        let first_line = RequestLine::from_utf8(it.next().unwrap());
        let mut headers: HashMap<&str, String> = HashMap::new();
        while let Some(header) = it.next() {
            let mut line = header.trim().split(": ");
            let name = line.next();
            let value = line.next();
            if name.is_none() || value.is_none() {
                break;
            }
            headers.insert(name.unwrap(), value.unwrap().clone().to_string());
        }
		let mut data: Option<String> = None;
		if first_line.has_body() {
			let mut buf = String::new();
			while let Some(line) = it.next() {
				buf += line;
			}
			data = Some(buf);
		}
        Self {
            request_line: first_line,
            host: headers.get("Host").unwrap_or(&"".to_string()).clone(),
            user_agent: headers.get("User-Agent").unwrap_or(&"".to_string()).clone(),
            accept: headers.get("Accept").unwrap_or(&"".to_string()).clone(),
            proxy_connection: headers.get("Proxy-Connection").unwrap_or(&"".to_string()).clone(),
			data,
        }
    }
}

pub async fn handle_http(msg: &Message) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
	// 取出 Host
	let text = std::str::from_utf8(msg.body.as_slice())
		.unwrap()
		.trim_matches('\u{0}')
		.to_string();
	debug!("{}", text);
	let req = HttpRequest::from_utf8(text.as_str());

	if req.request_line.url == "Unknown" {
		return Err("Unknown request".into());
	}

	info!("Redirect request to {:?}", req.request_line.url);

	// Check the method
	if !req.request_line.is_supported() {
		return Err(format!("Don't support {:?} method now", req.request_line.method).into());
	}

	// Create client with timeout
	let client = match ClientBuilder::new().timeout(Duration::from_secs(GATEWAY_TIMEOUT)).build() {
		Ok(client) => client,
		Err(err) => {
			return Err(Box::new(err));
		}
	};
	// 转发给指定的 Host
	let mut request = client.get(req.request_line.url.clone());
	// Support post
	if req.request_line.has_body() {
		let s = req.data.unwrap_or("".to_string());
		let data = s.as_str().clone();
		request = client.post(req.request_line.url.clone())
			.header("Content-Type", "application/json")
			.body(format!("{:?}", data));
	}
	let body = match request.send().await {
		Ok(json) => json,
		Err(err) => {
			return Err(format!("Request send failed: {:?}", err).into());
		}
	};
	
	let mut res_text = String::new();
	res_text += &format!("{:?} {:?}\r\n", body.version(), body.status().as_u16());
	let specify = "";
	for (key, value) in body.headers() {
		let mut v = value.to_str().unwrap().to_string();
		if key.to_string() == "content-length" {
			let mut size = v.parse::<usize>().unwrap();
			size += specify.len();
			v = format!("{:?}", size)
		}
		res_text += &format!(
			"{}: {}\r\n",
			key.to_string(),
			v
		)
	}
	let text = match body.text().await {
		Ok(text) => text,
		Err(err) => {
			return Err(Box::new(err));
		}
	};
	res_text += &format!("\r\n{}{} \r\n", specify, text);
	debug!("{}", res_text);
	Ok(res_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_request() {
        let http_req = "GET http://baidu.com/ HTTP/1.1\r\nHost: baidu.com\r\nUser-Agent: curl/7.71.1\r\nAccept: */*\r\nProxy-Connection: Keep-Alive\r\n\r\n";
        let req = HttpRequest::from_utf8(http_req);
        println!("{:?}", req);
    }
}
