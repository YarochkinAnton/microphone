# Microphone

Scream notifications let the sound guy do the rest

## Why?

1) I need to practice building stuff
2) I don't want to use SMTP for notifications

## What?

Web service that can receive messages for different topics from specified hosts or networks
and send them to configured recipients through Telegram bot.

Area of operation: my private networks.

---

Creating a bot and allowing it to send messages to users is out of the scope of this project

---

### Imagine my router running keepalived changes state from BACKUP to MASTER

How will it notify me?

Like this:

``` http
POST /myLab/edge-router-01 HTTP/2.0
Host: microphone
Content-Type: text/plain

My state has changed.
I am MASTER now!
```

Some headers ommited

or in curl's terms:

```bash
read -r -d '' MESSAGE <<EOF
My state has changed.
I am MASTER now!
EOF

curl -X POST "http://microphone/myLab/edge-router-01" \
    --data $MESSAGE
```

This should be displayed like this:

---

From: **edge-router-01@myLab**

My state has changed.

I am MASTER now!

---

### What about security?

Because this service will be mainly running inside my private network I think using ip address
based security will be enough

Using ip address based security will keep me from accidentally posting messages to incorrect
topics

### Configuration

Let's look at example configuration

``` toml
# If you're not familiar with TOML format
# Please refer to https://toml.io

# Port that the service will listen to
port = 80

# Telegram bot token that you'll get after bot creation with @BotFather
secret = "Scrape some shit up off a public toilet and eat it!"

[topics.myLab]
# List of string containing recipient IDs
# Refer to https://core.telegram.org/bots/api#sendmessage [chat_id]
# and @myidbot
recipients = [
    "11111111"
]
# List of IPs with network masks in CIDR notation
# If you want to know more about CIDR
# Refer to https://en.wikipedia.org/wiki/Classless_Inter-Domain_Routing
allow_list = [
    "192.168.69.0/24"
]
```

With this configuration any host from `192.168.69.0/24` subnet can post a message for `myLab`
and it will be forwarded to Telegram user with id `11111111`

## Building

To build this project you will need:

- git
- [rust](https://rustup.rs/)

Clone this repo:

```sh
git clone https://github.com/YarochkinAnton/microphone.git
```

Change working directory and build it

```sh
cd microphone
cargo build --release
```

You can find resulting binary at `./target/release/microphone` relative to the project root

## Usage

### Launching

```sh
./microphone /path/to/config.toml
```

### Sending text message

```sh
curl -X POST "http://localhost/topic/sender" \
    --data "Some text"
```

### Sending file without text

```sh
curl -X POST "http://localhost/topic/sender" \
    --form "file=@some_file.txt"
```

### Sending file with text

```sh
curl -X POST "http://localhost/topic/sender" \
    --form "file=@some_file.txt" \
    --form "message=Some text"
```
