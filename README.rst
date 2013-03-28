Recency Tracker
===============

A client+server combo that's supposed to provide a very simple
API to forward information to clients about the recency of resources
they're interested in.


Usage (client/browser)
----------------------

::

    <!-- include sock.js and resources/static/recency-tracker.js -->

    <div id="js-comments" data-version="123456">.. some comments here ..</div>

    <script type="text/javascript">
        var tracker = new RecencyTracker("http://subscriber-host:port/subscribe"),
            resource = "comments-for-something",
            currentVersion = $('#js-comments').data('version');

        tracker.subscribe(resource, currentVersion, function (newVersion) {
            //The server says that there is a more recent version (`newVersion`).

            //Let's run some code that updates it.
            $.getJSON("/comments", function (response) {
                $('#js-comments').html(response.html);
            });
        });
    </script>


Usage (server/PHP)
------------------

::

    function addComment($text) {
        addCommentToDatabase($text);

        $resource = 'comments-for-something';
        $commentsVersion = time();

        announceUpdate($resource, $commentsVersion);
    }

    function announceUpdate($resource, $version) {
        //Make a request to `http://publisher-host:port/publish`,
        //to tell it that $version is now the latest version for the resource $resource
    }
