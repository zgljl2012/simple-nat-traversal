use std::collections::HashMap;


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
            method: r.next().unwrap().to_string(),
            url: r.next().unwrap().to_string(),
            http_version: r.next().unwrap().to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub request_line: RequestLine,
    pub host: String,
    pub user_agent: String,
    pub accept: String,
    pub proxy_connection: String
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
        Self {
            request_line: first_line,
            host: headers.get("Host").unwrap_or(&"".to_string()).clone(),
            user_agent: headers.get("User-Agent").unwrap_or(&"".to_string()).clone(),
            accept: headers.get("Accept").unwrap_or(&"".to_string()).clone(),
            proxy_connection: headers.get("Proxy-Connection").unwrap_or(&"".to_string()).clone(),
        }
    }
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
