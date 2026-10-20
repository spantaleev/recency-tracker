Recency Tracker
===============

A client + server combo that helps you keep resources up-to-date in realtime.

A resource could be:
 * a comments list on a news article page
 * the news article's text
 * the article page as a whole
 * anything else that changes on your site and you want to keep it up-to-date in realtime

This system only deals with notifications about changes to resources.
What happens after you receive the change-notification is up to you.


Actions and actors
------------------

 * **Tracking** - the **notification server** tracks **resources** and their **versions**

 * **Publishing** - your **application server** publishes updates to the notification server when a resource gets updated

 * **Subscribing** - the **client's browser** subscribes to the **notification server** for updates


**Tracking** is performed by the notification server. See **Installation** below.

**Publishing** is done with a simple POST request from your application server. Check `examples/_publisher.html` to see how to do it yourself.
There is a [PHP library](https://github.com/spantaleev/recency-tracker-publisher-php) to make it easier in PHP applications.

**Subscribing** happens in the client's browser with the help of the included `recency-tracker.js` library and the browser's native WebSocket support. No additional browser library is required.


High-level view / example
-------------------------

Imagine a system which isn't good at doing realtime updates (like a PHP application).

You render a page with a comments list on it and send it to the client's browser.
You **can** provide a URL that allows the client to make an AJAX request and fetch an updated comments list.

If you only had a way of **figuring out when** the comments list has a new version, so that you could make that AJAX request and update the list.

That's the problem it solves.
It allows your PHP (or other) application to publish such updates, which then get pushed to each interested client's browser.
What you do with the notification message in the browser is up to you (updating a comments list in the above example).


### Subscribing

	<script type="text/javascript" src="http://subscriber-host:port/recency-tracker.js"></script>

    <div id="js-comments" data-version="1">.. some comments here ..</div>

    <script type="text/javascript">
        var tracker = new RecencyTracker("http://subscriber-host:port/subscribe"),
            resource = "/news/38/comments",
            currentVersion = $('#js-comments').data('version');

        tracker.subscribe(resource, currentVersion, function (newVersion) {
            //The server says that there is a more recent version (`newVersion`).
            //Let's run some code that updates the comments list.

            $.getJSON("/comments", function (response) {
                $('#js-comments').html(response.html);
            });
        });
    </script>

Flow:
 * the clients receives a news page with a pre-rendered comments list (call it version 1)
 * the client subscribes for notifications about resource `/news/38/comments` (comments for news item 38)
 * the client receives a notification when version 2 or higher becomes available
 * the client does whatever's needed to update the comments list (an AJAX request to fetch a new list in this case)

The constructor accepts `ws://`, `wss://`, `http://`, `https://`, or relative URLs.
HTTP URLs are converted to their WebSocket equivalents; use HTTPS/WSS when the
page is served over HTTPS. Relative URLs resolve against the page URL.

The client reconnects after five seconds when a connection closes and subscribes
again using the last received version of each resource. Removing the last
subscription closes the connection and cancels any pending reconnect.


### Publishing (PHP)

    function addComment($newsId, $text) {
        addCommentToDatabase($newsId, $text);

        $resource = '/news/' . $newsId . '/comments';
        $version = time();

        announceUpdate($resource, $version);
    }

    function announceUpdate($resource, $version) {
        //Make a request to `http://publisher-host:port/publish`,
        //to tell it that $version is now the latest version of the resource $resource
        //
        //See examples/_publisher.html for a simple implementation of a publisher.
    }

Flow:
 * the user submits a new comment
 * the application server stores the comment in the database
 * the application server tells the notification server about a change to the comments list
 * the notification server tells all subscribed clients (browsers) about the change


Resources
---------

A resource has a **name** (any string) and a **version** (string or integer).

Name examples: `/news/123/comments`, `/news/123/text`.
Version examples: `1365017905` (unix timestamp), `45765` (your own integer increment), `2013-04-25 15:48:19` (some datestamp, high to low), `515c8559499c215c3d000000` (MongoId)

Resource versions **MUST** be increasing for clients to receive an update.
Publishing version "3" will not update a client that has seen "5" before.
That is to say, the act of publishing is only meaningful if you submit an incremented version "number".


Publishing
----------

When publishing an update for a resource, you can **optionally** submit additional data to be passed to clients.
See `examples/example.html` to learn more.


Tracking & Persistence
----------------------

By default, the tracking server is configured to persist information about resources (and their versions) to the filesystem.
Additional methods of persistence can be added later ([MongoDB](http://mongodb.org/) is a good next choice).

This means that the server can be restarted at any time without losing its previous state.
Naturally, while it's down, your application server cannot inform it about changes to resources (publishing new updates).

The only way to get around that limitation is to have the application server know how to publish and persist the updates itself.


Installation
------------

### Container (recommended for self-hosting)

Images are published to `ghcr.io/spantaleev/recency-tracker` for Linux amd64 and
arm64. Use an exact release tag such as `2.0.0` for deliberate upgrades. The `2.0`,
`2`, and `latest` tags follow stable releases; `edge` follows the development branch.

Copy `config/container.json.dist` to a local `config.json` and **change the publishing
secret**, or supply it through `RECENCY_TRACKER_PUBLISHER_SECRET`. This configuration
binds both servers to the container network and stores resource files in `/data`.
The image requires a configuration mount; it does not include a default secret.

Example Compose service, with a private publishing endpoint and a subscriber port
bound to localhost for a reverse proxy:

```yaml
services:
  recency-tracker:
    image: ghcr.io/spantaleev/recency-tracker:2.0.0
    restart: unless-stopped
    read_only: true
    ports:
      - "127.0.0.1:9999:9999"
    volumes:
      - ./config.json:/config/config.json:ro
      - ./data:/data
```

Create the host `data` directory before starting the service, and make it writable
by UID/GID `1000:1000` (the image's default `node` user). An explicit Compose `user`
can select a different UID/GID; that user must be able to read the configuration
and write the storage directory **and any existing resource files**. The image runs
directly as Node.js and works with a read-only root filesystem.

Applications on the same Compose network publish to
`http://recency-tracker:9998/publish`. Keep that port private to applications that
know the secret; it does not need a host port mapping. If the publisher runs on
the host, add `127.0.0.1:9998:9998`. Put TLS and WebSocket proxying in front of the
subscriber endpoint for public browser connections (see the upgrade notes below).
Both servers offer `/ping` for HTTP health checks.

To migrate existing storage, mount the directory containing the resource `.json`
files at `/data`; no file conversion is needed. Stop the old tracker before starting
its replacement, and deploy a matching browser client when upgrading from v1.
The browser client is served at `/recency-tracker.js`, or can be copied from
`resources/static/recency-tracker.js` into your application's assets.

To build and check the image locally:

```sh
docker build -t recency-tracker:local .
node scripts/smoke-container.js recency-tracker:local
```

The smoke check uses temporary storage and containers. It verifies authenticated
publishing, WebSocket delivery, persistence after replacing the container, read-only
mounts, and an arbitrary runtime UID. Node.js 22 or newer and Docker are required.

### Running directly with Node.js

 * Make sure you have [Node.js](https://nodejs.org/) 22 or newer installed
 * Clone this repository
 * Run `npm install` to install the required dependencies
 * Copy `config/config.json.dist` to `config/config.json`
 * Customize the configuration in `config/config.json` - be sure to **change the publishing secret**
 * Run `npm start` to start the server

In practice, you may wish to run the server process under a process manager, such as [supervisord](http://supervisord.org/).
For a sample configuration, see `resources/supervisord-config.conf`.

Set `RECENCY_TRACKER_CONFIG` to load a configuration file from a different path.
Otherwise, the server reads `config/config.json` from its working directory.
`RECENCY_TRACKER_PUBLISHER_SECRET`, when set, overrides `publisher.secret` in that
file without changing it. An explicitly empty environment secret prevents startup.


Testing
-------

Run `npm test` after installing dependencies. Tests use Node.js's built-in test runner
and temporary local HTTP/WebSocket servers; no running tracker or configuration file is required.
They cover publishing, subscriptions, browser reconnect behavior, connection cleanup,
and catching up from filesystem storage.


Upgrading from 1.x to 2.x
------------------------

Version 2 replaces SockJS with native WebSockets. The browser API
(`subscribe(resource, version, callback)` and `unsubscribe(resource)`), publishing
endpoint and secret, configuration format, and persisted resource files are unchanged.

The subscription transport is incompatible with 1.x: **deploy the new server and
new browser client together**. Existing SockJS clients cannot connect to the v2
server, and the v2 browser client cannot connect to a v1 SockJS server.

 * Use Node.js 22 or newer for the server.
 * Replace any copied `recency-tracker.js` with the v2 file and update its asset
   version or URL so browsers fetch it. Open pages running the old client must reload.
 * Remove the SockJS script and any dependency on it in your script loader.
 * Existing subscription URLs such as `https://tracker.example/subscribe` can stay:
   the new client converts them to `wss://` automatically.
 * Configure reverse proxies to forward WebSocket upgrades at `/subscribe`,
   including the `Upgrade` and `Connection` headers. Allow an idle timeout longer
   than the server's 30-second ping interval.
   For nginx, explicitly setting `proxy_http_version 1.1` also supports versions
   before 1.29.7, whose upstream default was HTTP/1.0. See
   [nginx's WebSocket documentation](https://nginx.org/en/docs/http/websocket.html).
 * HTTP polling and streaming fallbacks are removed. Browsers and proxies must
   support WebSockets. Each client-to-server message is limited to 64 KiB.

Pinned deployments keep using their existing code until the source revision or
image is deliberately upgraded. For an upgrade, keep the matching v1 server image
and browser asset available so both sides can be rolled back together. Resource
storage requires no migration.

### Subscription protocol

Connect a WebSocket directly to `/subscribe`. Each text frame contains a JSON message,
without SockJS framing:

```json
{"type":"subscribe","resource":"/news/38/comments","version":"1"}
{"type":"unsubscribe","resource":"/news/38/comments"}
```

Updates arrive as:

```json
{"type":"update","resource":"/news/38/comments","version":"2","data":{}}
```

Resource names must start with `/`. Subscription versions may be strings or finite
numbers; update versions are strings. Invalid subscription commands are ignored.
Malformed JSON closes the connection with code 1007, binary messages with 1003,
and oversized messages with 1009. The server pings connections every 30 seconds
and drops those that have not answered the previous ping; browsers answer pings
automatically.
