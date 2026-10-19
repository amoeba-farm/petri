use crate::research::model::{Post, Snapshot};
use reqwest::blocking::Client;
use serde_json::Value;
use std::{io::Read, sync::mpsc, thread, time::Duration};

pub enum Command {
    Snapshot(Option<String>),
    Post(Post),
}
pub enum Reply {
    Snapshot(Result<Snapshot, String>),
    Post(Result<(), String>),
}

pub struct Api {
    client: Client,
    base: String,
    token: String,
}
impl Api {
    pub fn new(base: String, token: String) -> Result<Self, String> {
        let url = reqwest::Url::parse(&base).map_err(|_| "Invalid workspace address.")?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || !(url.scheme() == "https"
                || url.scheme() == "http"
                    && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")))
        {
            return Err("The workspace needs HTTPS or a loopback address.".into());
        }
        if token.len() != 64 || !token.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("Open Petri through its launcher.".into());
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "Could not start the workspace client.")?;
        Ok(Self {
            client,
            base: base.trim_end_matches('/').into(),
            token,
        })
    }
    fn request(&self, path: &str, body: Option<&Value>) -> Result<Value, String> {
        let url = format!("{}{path}", self.base);
        let mut builder = if let Some(body) = body {
            self.client.post(url).json(body)
        } else {
            self.client.get(url)
        };
        builder = builder
            .bearer_auth(&self.token)
            .header("Cache-Control", "no-store");
        let response = builder.send().map_err(|_| "Research workspace offline.")?;
        let status = response.status();
        let mut bytes = Vec::new();
        response
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "The workspace response could not be read.")?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("The workspace response is too large.".into());
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid workspace response.")?;
        if !status.is_success() || value["ok"] != true {
            return Err(crate::research::model::safe(
                value["error"]["message"]
                    .as_str()
                    .unwrap_or("The workspace could not complete this request."),
            ));
        }
        value
            .get("data")
            .cloned()
            .ok_or_else(|| "Incomplete workspace response.".into())
    }
    pub fn snapshot(&self, agent: Option<&str>) -> Result<Snapshot, String> {
        let path = match agent {
            Some(id) if id.len() == 64 && id.chars().all(|c| c.is_ascii_hexdigit()) => {
                format!("/v1/owner/snapshot?agentId={id}")
            }
            _ => "/v1/owner/snapshot".into(),
        };
        serde_json::from_value(self.request(&path, None)?)
            .map_err(|_| "Invalid workspace data.".into())
    }
    pub fn post(&self, post: &Post) -> Result<(), String> {
        self.request(post.path, Some(&post.body)).map(|_| ())
    }
}

pub fn worker(api: Api) -> (mpsc::Sender<Command>, mpsc::Receiver<Reply>) {
    let (commands, incoming) = mpsc::channel();
    let (outgoing, replies) = mpsc::channel();
    thread::spawn(move || {
        while let Ok(command) = incoming.recv() {
            let result = match command {
                Command::Snapshot(id) => Reply::Snapshot(api.snapshot(id.as_deref())),
                Command::Post(post) => Reply::Post(api.post(&post)),
            };
            if outgoing.send(result).is_err() {
                break;
            }
        }
    });
    (commands, replies)
}
