use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Arc,
};

use actix_web::{
    dev::ConnectionInfo,
    guard,
    http::header,
    middleware::Logger,
    web::{
        self,
        PayloadConfig,
    },
    App,
    HttpResponse,
    HttpServer,
    Responder,
};
use futures::StreamExt;
use ipnet::IpNet;
use reqwest::{
    multipart::{
        Form,
        Part,
    },
    ClientBuilder,
    StatusCode,
};
use serde::{
    Deserialize,
    Serialize,
};

const TELEGRAM_API_BASE_URL: &str = "https://api.telegram.org";
const TELEGRAM_SEND_MESSAGE_METHOD: &str = "sendMessage";
const TELEGRAM_SEND_DOCUMENT_METHOD: &str = "sendDocument";
const TELEGRAM_MARKDOWN_V2_PARSE_MODE: &str = "MarkdownV2";

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
    http_client:      reqwest::Client,
    base_request_url: String,
}

impl TgClient {
    pub fn new(secret: String) -> Self {
        let http_client = ClientBuilder::new()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent("reqwest")
            .build()
            .expect("Failed to build http client");

        let base_request_url = format!("{}/bot{}", TELEGRAM_API_BASE_URL, secret);

        Self {
            http_client,
            base_request_url,
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
            .post(&format!(
                "{}/{}",
                self.base_request_url, TELEGRAM_SEND_MESSAGE_METHOD
            ))
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

    async fn send_message_to_all(
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

    async fn send_document(
        &self,
        recipient: &str,
        topic: &str,
        sender: &str,
        message: &str,
        filename: &str,
        file_content: &[u8],
    ) -> Result<reqwest::Response, reqwest::Error> {
        let caption = format!(
            "From: *{}@{}*\n\n{}",
            *TgMarkdownString::new(sender),
            topic,
            message
        );

        let form = Form::new()
            .text("chat_id", recipient.to_owned())
            .text("caption", caption)
            .text("parse_mode", TELEGRAM_MARKDOWN_V2_PARSE_MODE)
            .part(
                "document",
                Part::bytes(file_content.to_owned()).file_name(filename.to_owned()),
            );

        self.http_client
            .post(&format!(
                "{}/{}",
                self.base_request_url, TELEGRAM_SEND_DOCUMENT_METHOD
            ))
            .multipart(form)
            .send()
            .await
    }

    async fn send_document_to_all(
        &self,
        recipients: &[String],
        topic: &str,
        sender: &str,
        message: &str,
        filename: &str,
        file_content: &[u8],
    ) -> Vec<Result<reqwest::Response, reqwest::Error>> {
        futures::future::join_all(
            recipients
                .iter()
                .map(|recipient| {
                    self.send_document(recipient, topic, sender, message, filename, file_content)
                })
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
            parse_mode: TELEGRAM_MARKDOWN_V2_PARSE_MODE,
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

    const MAIN_RESOURCE_PATH: &str = "/{topic_name}/{sender}";

    HttpServer::new(move || {
        App::new()
            .wrap(Logger::default())
            .app_data(topics_data.clone())
            .app_data(tg_data.clone())
            .app_data(PayloadConfig::new(50 * 1000 * 1000))
            .service(
                web::resource(MAIN_RESOURCE_PATH)
                    .guard(guard::fn_guard(|ctx| {
                        ctx.header::<header::ContentType>()
                            .map(|val| val.0.to_string().contains("multipart/form-data"))
                            .unwrap_or(false)
                    }))
                    .route(web::post().to(post_message_with_document)),
            )
            .service(
                web::resource(MAIN_RESOURCE_PATH)
                    .guard(guard::Header("Content-Type", "text/plain"))
                    .route(web::post().to(post_message)),
            )
    })
    .workers(1)
    .bind(("0.0.0.0", config.port))?
    .run()
    .await
}

#[derive(Deserialize)]
struct PostPathData {
    topic_name: String,
    sender:     String,
}

fn extract_client_address(connection_info: ConnectionInfo) -> Result<IpAddr, HttpResponse> {
    let client_address =
        if let Some(ip_address_string) = connection_info.realip_remote_addr() {
            match ip_address_string.parse() {
                Ok(ip_address) => ip_address,
                Err(_) =>
                    return Err(HttpResponse::InternalServerError()
                        .body("Cannot parse ip address from string")),
            }
        } else {
            return Err(HttpResponse::InternalServerError()
                .body("Cannot get ip address string from request"));
        };

    Ok(client_address)
}

async fn post_message(
    connection_info: ConnectionInfo,
    topics: web::Data<Arc<Topics>>,
    tg_client: web::Data<Arc<TgClient>>,
    post_query: web::Path<PostPathData>,
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
                .send_message_to_all(
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

async fn post_message_with_document(
    connection_info: ConnectionInfo,
    topics: web::Data<Arc<Topics>>,
    tg_client: web::Data<Arc<TgClient>>,
    path_data: web::Path<PostPathData>,
    mut multipart: actix_multipart::Multipart,
) -> impl Responder {
    let client_address = match extract_client_address(connection_info) {
        Ok(client_address) => client_address,
        Err(err_response) => return err_response,
    };

    let mut message: Option<String> = None;
    let mut file_content = Vec::new();
    let mut filename = String::new();

    while let Some(item) = multipart.next().await {
        let mut field = match item {
            Ok(field) => field,
            Err(err) => return HttpResponse::BadRequest().body(err.to_string()),
        };

        match field.name() {
            "message" => {
                let mut message_bytes_buffer: Vec<u8> = Vec::new();
                while let Some(chunk) = field.next().await {
                    let chunk_bytes = match chunk {
                        Ok(chunk_bytes) => chunk_bytes,
                        Err(err) => return HttpResponse::BadRequest().body(err.to_string()),
                    };

                    message_bytes_buffer.extend(chunk_bytes);
                }

                message = match String::from_utf8(message_bytes_buffer) {
                    Ok(message) => Some(message),
                    Err(_) => return HttpResponse::BadRequest().body("Message is not valid UTF-8"),
                }
            }
            "file" => {
                filename = match field.content_disposition().get_filename() {
                    Some(filename) => filename.to_owned(),
                    None => return HttpResponse::BadRequest().body("Multipart filename missing"),
                };
                while let Some(chunk) = field.next().await {
                    match chunk {
                        Ok(chunk_bytes) => file_content.extend(chunk_bytes),
                        Err(err) => return HttpResponse::BadRequest().body(err.to_string()),
                    }
                }
            }
            field_name =>
                return HttpResponse::BadRequest()
                    .body(format!("Unexpected mutlipart field \"{}\"", field_name)),
        };
    }

    let message = message.unwrap_or_default();

    if file_content.is_empty() {
        return HttpResponse::BadRequest().body("Multipart no file provided");
    }

    match topics.get(&path_data.topic_name) {
        Some(topic_info) if topic_info.is_allowed(client_address) => {
            let responses = tg_client
                .send_document_to_all(
                    &topic_info.recipients,
                    &path_data.topic_name,
                    &path_data.sender,
                    &message,
                    &filename,
                    &file_content,
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
