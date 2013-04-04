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

**Subscribing** happens in the client's browser with the help of the included `recency-tracker.js` library (and [SockJS](http://sockjs.org/) in the background).


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

	<script type="text/javascript" src="http://cdn.sockjs.org/sockjs-0.3.min.js"></script>
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

 * Make sure you have [nodejs](http://nodejs.org/) installed
 * Clone this repository
 * Run `npm install` to install the required dependencies
 * Run `npm install MD5@1.0` if you plan on using the filesystem persister (enabled by default)
 * Copy `config/config.json.dist` to `config/config.json`
 * Customize the configuration in `config/config.json` - be sure to **change the publishing secret**
 * Run `npm start` to start the server

In practice, you may wish to run the server process under a process manager, such as [supervisord](http://supervisord.org/).
For a sample configuration, see `resources/supervisord-config.conf`.
