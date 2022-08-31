# Microphone

Scream notifications let the sound guy do the rest

## Why?

1) I need to practice building stuff
2) I don't want to use SMTP for notifications

## What?

Web service that can receive messages for different topics from specified entities.

Area of operation: my private networks.

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

or in curl's terms:

```bash
read -r -d '' MESSAGE <<EOF
My state has changed.
I am MASTER now!
EOF

curl -X POST "http://microphone/myLab/edge-router-01" \
    --header "Content-type: text/plain" \
    --data $MESSAGE

```

This should be displayed like this:

---

From: **edge-router-01@myLab**

My state has changed.

I am MASTER now!

---

### What about security?

Because this service will be mainly run inside my private network I think using ip address
based security will be enough

Using ip address based security will be keeping me from accidentally posting message to incorrect
topic

Let's look at example of what configuration could be like:

``` toml
port = 80
secret = "Scrape some shit up off a public toilet and eat it!"
recipient_id = 11111111

[topics]
myLab = [
    "192.168.69.0/24"
]
```

With this configuration any host from `192.168.69.0/24` subnet can post a message for `myLab`
