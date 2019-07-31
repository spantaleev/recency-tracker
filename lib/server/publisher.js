const
	http = require('http'),
	express = require('express'),
	bodyParser = require('body-parser');

module.exports.createServer = function (tracker, publisherSecret) {
	var app = express();

	app.use(bodyParser);

	app.get('/ping', function (request, response) {
		response.send('publisher server up');
	});

	app.post('/publish', function (request, response) {
		if (! ('secret' in request.body) || ! ('resource' in request.body) || ! ('version' in request.body)) {
			console.log('Received malformed publish request.');
			return response.json({"ok": false, "error": "bad request"});
		}

		if (request.body.secret !== publisherSecret) {
			console.log('Received unauthorized publish request.');
			return response.json({"ok": false, "error": "bad secret"});
		}

		if ('data' in request.body) {
			try {
				var data = JSON.parse(request.body.data);
			} catch (e) {
				console.log('Received malform publish request data.');
				return response.json({"ok": false, "error": "bad data - invalid json"});
			}
		} else {
			var data = {};
		}

		tracker.publish(request.body.resource, request.body.version, data);
		return response.json({"ok": true});
	});

	return http.createServer(app);
};
