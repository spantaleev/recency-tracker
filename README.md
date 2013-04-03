Recency Tracker
===============

A client + server combo that provides a very simple API that helps notify clients about the recency (up-to-dateness) of resources.


Actions and actors
------------------

 * **Tracking** - the **notification server** (nodejs powered) tracks **resources** and their **versions**

 * **Publishing** - your **application server** publishes updates to the notification server when a resource gets updated

 * **Subscribing** - the **client's browser** (with the help of the included library) subscribes to the **notification server** for updates


**Publishing** is done by a simple POST request. Check `examples/_publisher.html` to see how to do it yourself.
Alternatively, there is a PHP library that can make it easier.


High-level view
---------------

Imagine a system which isn't good at doing realtime updates (like a PHP application).

You render a page with a comments list on it and send it to the client's browser.
You **can** provide a URL that allows the client to make an AJAX request and fetch an updated comments list.

If you only had a way of **figuring out when** the comments list has a new version,
so that you could make that AJAX request and update the list.

In essence, it allows your PHP (or other) application to publish such updates, which then get pushed to each interested client's browser.
What you do with the notification is up to you (updating a comments list in the above example).


Regular visitor flow:
 * receives a page with a pre-rendered comments list (call it version #1)
 * awaits an update notification (version #2 or higher)
 * does whatever's needed to update the comments list (an AJAX request to fetch a new list)


Comment publishing flow:
 * the user submits a new comment
 * the application server stores that comment in the database
 * the application server tells the notification server about that (causing the server to forward that to all clients)

Resources
---------

A resource has a **name** (any string, but often looks like a unix filesystem path) and a **version** (string or integer).

Names example: `/news/123/comments`, `/news/123/text`.
Versions example: `1365017905` (unix timestamp), `515c8559499c215c3d000000` (MongoId), `45765` (your own integer increment), `2013-04-25 15:48:19` (some date stamp, high to low)

Resource versions **MUST** be increasing for clients to receive an update.
Publishing version "3" will not update a client that has seen "5" before.
That is to say, the act of publishing is only meaningful if you submit an incremented version "number".


Publishing
----------

When publishing an update for a resource, you can **optionally** submit additional data that will be passed to clients.
This can be useful in assisting the client with updating itself.
See `examples/example.html` to learn more.


Usage (client/browser)
----------------------

	<script type="text/javascript" src="http://cdn.sockjs.org/sockjs-0.3.min.js"></script>
	<script type="text/javascript" src="recency-tracker.js"></script>

    <div id="js-comments" data-version="123456">.. some comments here ..</div>

    <script type="text/javascript">
        var tracker = new RecencyTracker("http://subscriber-host:port/subscribe"),
            resource = "comments-for-something",
            currentVersion = $('#js-comments').data('version');

        tracker.subscribe(resource, currentVersion, function (newVersion) {
            //The server says that there is a more recent version (`newVersion`).
            //Let's run some code that updates the comments list.

            $.getJSON("/comments", function (response) {
                $('#js-comments').html(response.html);
            });
        });
    </script>


Usage (server/PHP)
------------------

    function addComment($text) {
        addCommentToDatabase($text);

        $resource = 'comments-for-something';
        $commentsVersion = time();

        announceUpdate($resource, $commentsVersion);
    }

    function announceUpdate($resource, $version) {
        //Make a request to `http://publisher-host:port/publish`,
        //to tell it that $version is now the latest version of the resource $resource
        //
        //See examples/_publisher.html for a simple implementation of a publisher.
    }
