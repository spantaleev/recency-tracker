const
	http = require('http'),
	sockjs = require('sockjs'),
	express = require('express'),
	validator = require('../../lib/validator.js');

module.exports.createServer = function (tracker, staticResourcesPath) {
	var subscribe = sockjs.createServer();
	subscribe.on('connection', function (connection) {
		console.log('[+] Connection #' + connection.id);

		var client = null;

		connection.on('data', function (message) {
			var message = JSON.parse(message);
			if (message === null || typeof(message.type) !== 'string' || typeof(message.resource) !== 'string') {
				return;
			}

			if (!validator.isValidResourceName(message.resource)) {
				console.log('Invalid resource name: ' + message.resource);
				return;
			}

			if (client === null) {
				client = tracker.createClient(connection.id);

				client.events.on('outdated', function (resource, newVersion) {
					connection.write(JSON.stringify({
						"type": "update",
						"resource": resource,
						"version": newVersion
					}));
				});
			}

			if (message.type === 'subscribe') {
				if (typeof(message.version) === 'undefined') {
					return;
				}

				tracker.subscribe(client, message.resource, message.version);
			} else if (message.type === 'unsubscribe') {
				tracker.unsubscribe(client, message.resource);
			}
		});

		connection.on('close', function () {
			console.log('[-] Connection #' + connection.id);

			if (client !== null) {
				tracker.deleteClient(client);
				delete client;
			}
		});
	});

	var app = express();

	app.use(express.static(staticResourcesPath));

	var httpServer = http.createServer(app);

	subscribe.installHandlers(httpServer, {"prefix": "/subscribe"});

	return httpServer;
};
