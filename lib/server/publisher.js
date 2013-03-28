const
	http = require('http'),
	express = require('express');

module.exports.createServer = function (tracker, publisherSecret) {
	var app = express();

	app.use(express.bodyParser());

	app.post('/publish', function (request, response) {
		if (! ('secret' in request.body) || ! ('resource' in request.body) || ! ('version' in request.body)) {
			console.log('Received malformed publish request.');
			response.write(JSON.stringify({"ok": false, "error": "bad request"}));
			return response.end();
		}

		if (request.body.secret !== publisherSecret) {
			console.log('Received unauthorized publish request.');
			response.write(JSON.stringify({"ok": false, "error": "bad secret"}));
			return response.end();
		}

		tracker.publish(request.body.resource, request.body.version);
		response.write(JSON.stringify({"ok": true}));
		response.end();
	});

	return http.createServer(app);
};


