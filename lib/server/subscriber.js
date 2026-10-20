const
	http = require('http'),
	crypto = require('crypto'),
	{ WebSocket, WebSocketServer } = require('ws'),
	express = require('express'),
	validator = require('../../lib/validator.js');

module.exports.createServer = function (tracker, staticResourcesPath) {
	const app = express();
	app.use(express.static(staticResourcesPath));
	app.get('/ping', function (request, response) {
		response.send('subscriber server up');
	});

	const httpServer = http.createServer(app);
	const subscribe = new WebSocketServer({
		server: httpServer,
		path: '/subscribe',
		maxPayload: 64 * 1024,
		perMessageDeflate: false
	});

	subscribe.on('connection', function (connection) {
		const id = crypto.randomUUID();
		console.log('[+] Connection #' + id);
		let client = null;

		connection.isAlive = true;
		connection.on('pong', function () {
			connection.isAlive = true;
		});
		connection.on('error', function (error) {
			console.log('Connection #' + id + ' error: ' + error.message);
		});

		connection.on('message', function (data, isBinary) {
			if (isBinary) {
				connection.close(1003, 'Text messages required');
				return;
			}

			let message;
			try {
				message = JSON.parse(data.toString());
			} catch (error) {
				connection.close(1007, 'Invalid JSON');
				return;
			}

			if (message === null || typeof message !== 'object' ||
				!validator.isValidResourceName(message.resource)) {
				return;
			}

			if (message.type === 'unsubscribe') {
				if (client !== null) {
					tracker.unsubscribe(client, message.resource);
				}
				return;
			}

			if (message.type !== 'subscribe' ||
				!(typeof message.version === 'string' ||
					(typeof message.version === 'number' && Number.isFinite(message.version)))) {
				return;
			}

			if (client === null) {
				client = tracker.createClient(id);
				client.events.on('outdated', function (resource, newVersion, data) {
					if (connection.readyState === WebSocket.OPEN) {
						connection.send(JSON.stringify({
							type: 'update',
							resource: resource,
							version: newVersion,
							data: data
						}), function (error) {
							if (error) {
								connection.terminate();
							}
						});
					}
				});
			}

			tracker.subscribe(client, message.resource, message.version);
		});

		connection.on('close', function () {
			console.log('[-] Connection #' + id);
			if (client !== null) {
				tracker.deleteClient(client);
			}
		});
	});

	// Detect broken connections and keep idle WebSocket proxy connections active.
	const heartbeat = setInterval(function () {
		for (const connection of subscribe.clients) {
			if (!connection.isAlive) {
				connection.terminate();
				continue;
			}
			connection.isAlive = false;
			connection.ping();
		}
	}, 30000);
	heartbeat.unref();

	// HTTP server shutdown does not close upgraded sockets on its own.
	const close = httpServer.close;
	httpServer.close = function (callback) {
		clearInterval(heartbeat);
		subscribe.close();
		for (const connection of subscribe.clients) {
			connection.terminate();
		}
		return close.call(this, callback);
	};

	return httpServer;
};
