use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Arc,
};

use actix_web::{
    dev::ConnectionInfo,
    middleware::Logger,
    web,
    App,
    HttpResponse,
    HttpServer,
    Responder,
};
use ipnet::IpNet;
use reqwest::{
    ClientBuilder,
    StatusCode,
};
use serde::{
    Deserialize,
    Serialize,
};

const TELEGRAM_API_BASE_URL: &str = "https://api.telegram.org";
const TELEGRAM_SEND_MESSAGE_METHOD: &str = "sendMessage";
const TELEGRAM_SEND_MESSAGE_PARSE_MODE: &str = "MarkdownV2";

type Topics = HashMap<String, Topic>;

#[derive(Deserialize)]
struct Config {
    port:   u16,
    secret: String,
    topics: Topics,
}

#[derive(Debug)]
#[derive(Deserialize)]
#[derive(Clone)]
struct Topic {
    recipients: Vec<String>,
    allow_list: Vec<IpNet>,
}

impl Topic {
    pub fn is_allowed(&self, address: IpAddr) -> bool {
        self.allow_list.iter().any(|allow| allow.contains(&address))
    }
}

struct TgClient {
    http_client: reqwest::Client,
    request_url: String,
}

impl TgClient {
    pub fn new(secret: String) -> Self {
        let http_client = ClientBuilder::new()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent("reqwest")
            .build()
            .expect("Failed to build http client");

        let request_url = format!(
            "{}/bot{}/{}",
            TELEGRAM_API_BASE_URL, secret, TELEGRAM_SEND_MESSAGE_METHOD
        );

        Self {
            http_client,
            request_url,
        }
    }

    async fn send_message(
        &self,
        recipient: &str,
        topic: &str,
        sender: &str,
        text: &str,
    ) -> Result<reqwest::Response, reqwest::Error> {
        self.http_client
            .post(&self.request_url)
            .json(&SendMessagePayload::new(
                recipient,
                &format!(
                    "From: *{}@{}*\n\n{}",
                    *TgMarkdownString::new(sender),
                    topic,
                    text
                ),
            ))
            .send()
            .await
    }

    async fn notify_all(
        &self,
        recipients: &[String],
        topic: &str,
        sender: &str,
        text: &str,
    ) -> Vec<Result<reqwest::Response, reqwest::Error>> {
        futures::future::join_all(
            recipients
                .iter()
                .map(|recipient| self.send_message(recipient, topic, sender, text))
                .collect::<Vec<_>>(),
        )
        .await
    }
}

#[derive(Serialize)]
struct TgMarkdownString(String);

impl std::ops::Deref for TgMarkdownString {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl TgMarkdownString {
    pub fn new(s: &str) -> Self {
        let need_to_escape = [
            '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.',
            '!',
        ];
        let mut escaped_string = String::new();

        for ch in s.chars() {
            if need_to_escape.contains(&ch) {
                escaped_string.push('\\');
            }
            escaped_string.push(ch);
        }

        Self(escaped_string)
    }
}

#[derive(Serialize)]
struct SendMessagePayload<'a> {
    chat_id:    &'a str,
    parse_mode: &'static str,
    text:       String,
}

impl<'a> SendMessagePayload<'a> {
    pub fn new(chat_id: &'a str, text: &str) -> Self {
        Self {
            chat_id,
            text: text.to_owned(),
            parse_mode: TELEGRAM_SEND_MESSAGE_PARSE_MODE,
        }
    }
}

#[actix_web::main]
async fn main() -> Result<(), std::io::Error> {
    let config_path = std::env::args()
        .nth(1)
        .expect("Provide config file path as the first argument to the program");

    let config: Config =
        toml::from_str(&std::fs::read_to_string(config_path).expect("Failed to read config file"))
            .expect("Failed to parse config file");

    let topics_data = web::Data::new(Arc::new(config.topics.clone()));

    let tg_data = web::Data::new(Arc::new(TgClient::new(config.secret)));

    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));

    HttpServer::new(move || {
        App::new()
            .wrap(Logger::default())
            .app_data(topics_data.clone())
            .app_data(tg_data.clone())
            .service(post_message)
    })
    .workers(1)
    .bind(("0.0.0.0", config.port))?
    .run()
    .await
}

#[derive(Deserialize)]
struct PostQuery {
    topic_name: String,
    sender:     String,
}

#[actix_web::post("/{topic_name}/{sender}")]
async fn post_message(
    connection_info: ConnectionInfo,
    topics: web::Data<Arc<Topics>>,
    tg_client: web::Data<Arc<TgClient>>,
    post_query: web::Path<PostQuery>,
    message: String,
) -> impl Responder {
    let client_address: IpAddr =
        if let Some(ip_address_string) = connection_info.realip_remote_addr() {
            match ip_address_string.parse() {
                Ok(ip_address) => ip_address,
                Err(_) =>
                    return HttpResponse::InternalServerError()
                        .body("Cannot parse ip address from string"),
            }
        } else {
            return HttpResponse::InternalServerError()
                .body("Cannot get ip address string from request");
        };

    match topics.get(&post_query.topic_name) {
        Some(topic_info) if topic_info.is_allowed(client_address) => {
            let responses = tg_client
                .notify_all(
                    &topic_info.recipients,
                    &post_query.topic_name,
                    &post_query.sender,
                    &message,
                )
                .await;

            if responses.iter().all(|res| {
                res.as_ref()
                    .map_or_else(|_| false, |resp| resp.status() == StatusCode::OK)
            }) {
                HttpResponse::NoContent().finish()
            } else {
                HttpResponse::InternalServerError().body("bAdBaDnOtGoOd")
            }
        }
        _ => HttpResponse::NotFound().body("No such topic"),
    }
}
