use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Aria2Client {
    endpoint: String,
    secret: Option<String>,
    id_seq: AtomicU64,
    http: reqwest::Client,
}

impl Aria2Client {
    pub fn new(endpoint: String, secret: Option<String>) -> Self {
        Self {
            endpoint,
            secret,
            id_seq: AtomicU64::new(1),
            http: reqwest::Client::new(),
        }
    }

    async fn call(&self, method: &str, params: Vec<Value>) -> Result<Value, String> {
        let id = self.id_seq.fetch_add(1, Ordering::Relaxed).to_string();
        let mut full_params = Vec::new();
        if let Some(secret) = &self.secret {
            full_params.push(Value::String(format!("token:{}", secret)));
        }
        full_params.extend(params);
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": full_params
        });
        let resp = self
            .http
            .post(&self.endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;
        if let Some(err) = v.get("error") {
            return Err(err.to_string());
        }
        v.get("result").cloned().ok_or_else(|| "missing result".into())
    }

    pub async fn add_uri(
        &self,
        url: &str,
        dir: &str,
        out: &str,
    ) -> Result<String, String> {
        let options = json!({ "dir": dir, "out": out });
        let result = self
            .call("aria2.addUri", vec![json!([url]), options])
            .await?;
        result
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "gid not string".into())
    }

    pub async fn pause(&self, gid: &str) -> Result<(), String> {
        self.call("aria2.pause", vec![json!(gid)]).await.map(|_| ())
    }

    pub async fn unpause(&self, gid: &str) -> Result<(), String> {
        self.call("aria2.unpause", vec![json!(gid)]).await.map(|_| ())
    }

    pub async fn remove(&self, gid: &str) -> Result<(), String> {
        self.call("aria2.remove", vec![json!(gid)]).await.map(|_| ())
    }

    pub async fn tell_status(&self, gid: &str) -> Result<Value, String> {
        self.call(
            "aria2.tellStatus",
            vec![
                json!(gid),
                json!(["status", "totalLength", "completedLength", "downloadSpeed"]),
            ],
        )
        .await
    }
}
