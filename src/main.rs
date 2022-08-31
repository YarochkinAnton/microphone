use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Arc,
};

use actix_web::{
    dev::ConnectionInfo,
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

#[derive(Deserialize)]
struct Config {
    port:         u16,
    recipient_id: usize,
    secret:       String,
    topics:       HashMap<String, Vec<IpNet>>,
}

struct TgClient {
    recipient_id: usize,
    http_client:  reqwest::Client,
    request_url:  String,
}

impl TgClient {
    pub fn new(recipient_id: usize, secret: String) -> Self {
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
            recipient_id,
            http_client,
            request_url,
        }
    }

    async fn send_message(
        &self,
        topic: &str,
        sender: &str,
        text: &str,
    ) -> Result<reqwest::Response, reqwest::Error> {
        self.http_client
            .post(&self.request_url)
            .json(&SendMessagePayload::new(
                self.recipient_id,
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
struct SendMessagePayload {
    chat_id:    usize,
    parse_mode: &'static str,
    text:       String,
}

impl SendMessagePayload {
    pub fn new(chat_id: usize, text: &str) -> Self {
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

    let tg_data = web::Data::new(Arc::new(TgClient::new(config.recipient_id, config.secret)));

    HttpServer::new(move || {
        App::new()
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
    topic:  String,
    sender: String,
}

#[actix_web::post("/{topic}/{sender}")]
async fn post_message(
    connection_info: ConnectionInfo,
    topics: web::Data<Arc<HashMap<String, Vec<IpNet>>>>,
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

    match topics.get(&post_query.topic) {
        Some(allow_list)
            if allow_list
                .iter()
                .any(|allow| allow.contains(&client_address)) =>
        {
            let response = tg_client
                .send_message(&post_query.topic, &post_query.sender, &message)
                .await;

            match response {
                Ok(response) if response.status() == StatusCode::OK =>
                    HttpResponse::NoContent().finish(),
                Err(err) if err.is_timeout() =>
                    HttpResponse::GatewayTimeout().body("Telegram API timed out"),
                _ => HttpResponse::InternalServerError().body("Something bad happened"),
            }
        }
        _ => HttpResponse::Forbidden().body("Host isn't allowed to send messages on this topic"),
    }
}
